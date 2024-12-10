use std::process::{Command, Stdio, Child};
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex, Semaphore};

mod turn_tracker;
use turn_tracker::{TurnTracker, TurnTracking};
mod ai;
mod hivegame_bot_api;
use hivegame_bot_api::HiveGameApi;
mod config;
use config::{Config, BotConfig};
mod cli;

const BASE_URL: &str = "http://your-server.com";

#[derive(Clone)]
struct Bot {
    name: String,
    uri: String,
    api_key: String,
    ai_command: String,
    bestmove_command_args: String,
}

struct GameTurn {
    game_string: String,
    hash: u64,
    bot: BotConfig,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Parse command line arguments
    let cli = cli::Cli::parse();

    // Load configuration from specified file
    let config = Config::load_from(cli.config)?;
    let (sender, receiver) = mpsc::channel(config.queue_capacity);
    let receiver = Arc::new(Mutex::new(receiver));
    let semaphore = Arc::new(Semaphore::new(config.max_concurrent_processes));
    let active_processes = Arc::new(Mutex::new(Vec::new()));
    let turn_tracker = TurnTracker::new();
    
    let cleanup_tracker = turn_tracker.clone();
    tokio::spawn(async move {
        loop {
            // Clean up every 2 sec (Will increase later)
            tokio::time::sleep(Duration::from_secs(2)).await;
            cleanup_tracker.cleanup().await;
        }
    });
    
    // Spawn a producer task for each bot
    let mut producer_handles = Vec::new();
    for bot in config.bots {
        let producer_handle = tokio::spawn(producer_task(
            sender.clone(),
            turn_tracker.clone(),
            config.base_url.clone(),
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
    base_url: String,
    bot: BotConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let api = HiveGameApi::new(base_url);
    
    loop {
        match api.fake_get_games(&bot.uri, &bot.api_key).await {
            Ok(game_strings) => {
                for game_string in game_strings {
                    let hash = calculate_hash(&game_string);
                    
                    if turn_tracker.tracked(hash).await {
                        continue;
                    }

                    let turn = GameTurn {
                        game_string,
                        hash,
                        bot: bot.clone(),
                    };

                    turn_tracker.processing(hash).await;

                    if sender.send(turn).await.is_err() {
                        eprintln!("Failed to send turn to queue");
                        continue;
                    }
                }
            }
            Err(e) => eprintln!("Failed to fetch games for bot {}: {}", bot.name, e),
        }

        println!("Start new cycle in 1 sec");
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

    let child = match ai::spawn_process(&turn.bot.ai_command, &turn.bot.name) {
        Ok(child) => child,
        Err(e) => {
            eprintln!("Failed to spawn AI process for bot {}: {}", turn.bot.name, e);
            turn_tracker.processed(turn.hash).await;
            return;
        }
    };

    match ai::run_commands(child, &turn.game_string, &turn.bot.bestmove_command_args).await {
        Ok(bestmove) => {
            println!("Bot '{}' bestmove: '{}'", turn.bot.name, bestmove);
            // Here you can handle the bestmove (e.g., send it to the server)
        }
        Err(e) => {
            eprintln!("Error running AI commands for bot '{}': '{}'", turn.bot.name, e);
        }
    }

    turn_tracker.processed(turn.hash).await;
}

async fn cleanup_processes(active_processes: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>) {
    let mut processes = active_processes.lock().await;
    processes.retain(|handle| !handle.is_finished());
}
