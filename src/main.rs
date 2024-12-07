use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex, Semaphore};
use reqwest;

mod turn_tracker;
use turn_tracker::{TurnTracker, TurnTracking};

const MAX_CONCURRENT_PROCESSES: usize = 5;
const QUEUE_CAPACITY: usize = 1000;
const BASE_URL: &str = "http://your-server.com";

struct Bot {
    name: String,
    uri: String,
    api_key: String,
    bestmove_command: String,
}

struct GameTurn {
    game_string: String,
    hash: u64,
    bot: Bot,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let bots = vec![
        Bot {
            name: "Bot1".to_string(),
            uri: "/games/bot1".to_string(),
            api_key: "key1".to_string(),
            bestmove_command: "bot1_engine".to_string(),
        },
        Bot {
            name: "Bot2".to_string(),
            uri: "/games/bot2".to_string(),
            api_key: "key2".to_string(),
            bestmove_command: "bot2_engine".to_string(),
        },
    ];

    let (sender, receiver) = mpsc::channel(QUEUE_CAPACITY);
    let receiver = Arc::new(Mutex::new(receiver));
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_PROCESSES));
    let active_processes = Arc::new(Mutex::new(Vec::new()));
    let turn_tracker = TurnTracker::new();
    
    let cleanup_tracker = turn_tracker.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            cleanup_tracker.cleanup();
        }
    });
    
    // Spawn a producer task for each bot
    let mut producer_handles = Vec::new();
    for bot in bots {
        let producer_handle = tokio::spawn(producer_task(
            sender.clone(),
            turn_tracker.clone(),
            bot,
        ));
        producer_handles.push(producer_handle);
    }
    
    let consumer_handle = tokio::spawn(consumer_task(
        receiver,
        semaphore,
        active_processes,
        turn_tracker.clone(),
    ));

    // Wait for all producers and the consumer
    for handle in producer_handles {
        if let Err(e) = handle.await? {
            eprintln!("Producer error: {}", e);
        }
    }
    if let Err(e) = consumer_handle.await? {
        eprintln!("Consumer error: {}", e);
    }
    
    Ok(())
}

fn calculate_hash(game_string: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    use std::collections::hash_map::DefaultHasher;
    let mut hasher = DefaultHasher::new();
    game_string.hash(&mut hasher);
    hasher.finish()
}

async fn producer_task(
    sender: mpsc::Sender<GameTurn>,
    turn_tracker: TurnTracker,
    bot: Bot,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::Client::new();
    let url = format!("{}{}", BASE_URL, bot.uri);
    
    loop {
        let game_strings: Vec<String> = client.get(&url)
            .header("Authorization", format!("Bearer {}", bot.api_key))
            .send()
            .await?
            .json()
            .await?;

        for game_string in game_strings {
            let hash = calculate_hash(&game_string);
            
            if turn_tracker.tracked(hash) {
                continue;
            }

            let turn = GameTurn {
                game_string,
                hash,
                bot: Bot {
                    name: bot.name.clone(),
                    uri: bot.uri.clone(),
                    api_key: bot.api_key.clone(),
                    bestmove_command: bot.bestmove_command.clone(),
                },
            };

            turn_tracker.processing(hash);

            if sender.send(turn).await.is_err() {
                eprintln!("Failed to send turn to queue");
                continue;
            }
        }

        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn consumer_task(
    receiver: Arc<Mutex<mpsc::Receiver<GameTurn>>>,
    semaphore: Arc<Semaphore>,
    active_processes: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    turn_tracker: TurnTracker,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    loop {
        let mut rx = receiver.lock().await;
        if let Some(turn) = rx.recv().await {
            drop(rx);
            
            let handle = tokio::spawn(process_turn(
                turn,
                semaphore.clone(),
                turn_tracker.clone(),
            ));

            active_processes.lock().await.push(handle);
            cleanup_processes(active_processes.clone()).await;
        }
    }
}

async fn process_turn(
    turn: GameTurn,
    semaphore: Arc<Semaphore>,
    turn_tracker: TurnTracker,
) {
    let _permit = semaphore.acquire().await.expect("Failed to acquire semaphore");

    let mut child = Command::new(turn.bot.bestmove_command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to spawn command");

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin.write_all(turn.game_string.as_bytes())
            .expect("Failed to write to stdin");
    }

    let status = child.wait()
        .expect("Failed to wait for child process");

    println!("Process exited with status: {} for bot {}", status, turn.bot.name);
    
    turn_tracker.processed(turn.hash);
}

async fn cleanup_processes(active_processes: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>) {
    let mut processes = active_processes.lock().await;
    processes.retain(|handle| !handle.is_finished());
}