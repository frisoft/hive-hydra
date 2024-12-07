use std::process::{Command, Stdio};
use std::sync::Arc;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex, Semaphore};
use reqwest;

const MAX_CONCURRENT_PROCESSES: usize = 5;
const QUEUE_CAPACITY: usize = 1000;
const SERVER_URL: &str = "http://your-server.com/lines";
const HASH_RETENTION_PERIOD: Duration = Duration::from_secs(3600);

struct GameTurn {
    game_state: String,
    hash: u64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (sender, receiver) = mpsc::channel(QUEUE_CAPACITY);
    let receiver = Arc::new(Mutex::new(receiver));
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_PROCESSES));
    let active_processes = Arc::new(Mutex::new(Vec::new()));
    let hashes = Arc::new(Mutex::new(HashMap::new())); 
    
    let cleanup_hashes = hashes.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            cleanup_old_hashes(cleanup_hashes.clone()).await;
        }
    });
    
    let producer_handle = tokio::spawn(producer_task(sender, hashes.clone()));
    let consumer_handle = tokio::spawn(consumer_task(
        receiver,
        semaphore,
        active_processes,
    ));

    let (producer_result, consumer_result) = tokio::try_join!(producer_handle, consumer_handle)?;
    producer_result?;
    consumer_result?;
    Ok(())
}

fn calculate_hash(game_state: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    use std::collections::hash_map::DefaultHasher;
    let mut hasher = DefaultHasher::new();
    game_state.hash(&mut hasher);
    hasher.finish()
}

async fn cleanup_old_hashes(hashes: Arc<Mutex<HashMap<u64, Instant>>>) {
    let now = Instant::now();
    let mut hash_map = hashes.lock().await;
    hash_map.retain(|_, timestamp| now.duration_since(*timestamp) < HASH_RETENTION_PERIOD);
}

async fn producer_task(
    sender: mpsc::Sender<GameTurn>,
    hashes: Arc<Mutex<HashMap<u64, Instant>>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::Client::new();
    
    loop {
        let response = client.get(SERVER_URL)
            .send()
            .await?
            .text()
            .await?;

        for game_state in response.lines() {
            let hash = calculate_hash(game_state);
            
            if hashes.lock().await.contains_key(&hash) {
                continue;
            }

            let turn = GameTurn {
                game_state: game_state.to_string(),
                hash,
            };

            hashes.lock().await.insert(hash, Instant::now());

            if sender.send(turn).await.is_err() {
                hashes.lock().await.remove(&hash);
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
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    loop {
        let mut rx = receiver.lock().await;
        if let Some(turn) = rx.recv().await {
            drop(rx);
            
            let handle = tokio::spawn(process_turn(
                turn.game_state,
                turn.hash,
                semaphore.clone(),
            ));

            active_processes.lock().await.push(handle);
            cleanup_processes(active_processes.clone()).await;
        }
    }
}

async fn process_turn(
    game_state: String,
    _hash: u64,
    semaphore: Arc<Semaphore>,
) {
    let _permit = semaphore.acquire().await.expect("Failed to acquire semaphore");

    let mut child = Command::new("your-command")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to spawn command");

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin.write_all(game_state.as_bytes())
            .expect("Failed to write to stdin");
    }

    let status = child.wait()
        .expect("Failed to wait for child process");

    println!("Process exited with status: {}", status);
}

async fn cleanup_processes(active_processes: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>) {
    let mut processes = active_processes.lock().await;
    processes.retain(|handle| !handle.is_finished());
}
