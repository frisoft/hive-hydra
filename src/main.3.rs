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

trait TurnTracking {
    fn tracked(&self, turn: &GameTurn) -> bool;
    fn processing(&self, turn: &GameTurn);
    fn processed(&self, turn: &GameTurn);
    fn cleanup(&self);
}

struct TurnTracker {
    processing_turns: Arc<Mutex<HashMap<u64, Instant>>>,
    processed_turns: Arc<Mutex<HashMap<u64, Instant>>>,
}

impl TurnTracker {
    fn new() -> Self {
        TurnTracker {
            processing_turns: Arc::new(Mutex::new(HashMap::new())),
            processed_turns: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn clone(&self) -> Self {
        TurnTracker {
            processing_turns: self.processing_turns.clone(),
            processed_turns: self.processed_turns.clone(),
        }
    }
}

impl TurnTracking for TurnTracker {
    fn tracked(&self, turn: &GameTurn) -> bool {
        let processing = self.processing_turns.blocking_lock();
        let processed = self.processed_turns.blocking_lock();
        processing.contains_key(&turn.hash) || processed.contains_key(&turn.hash)
    }

    fn processing(&self, turn: &GameTurn) {
        self.processing_turns.blocking_lock()
            .insert(turn.hash, Instant::now());
    }

    fn processed(&self, turn: &GameTurn) {
        self.processing_turns.blocking_lock().remove(&turn.hash);
        self.processed_turns.blocking_lock()
            .insert(turn.hash, Instant::now());
    }

    fn cleanup(&self) {
        let now = Instant::now();
        let mut processed = self.processed_turns.blocking_lock();
        processed.retain(|_, timestamp| now.duration_since(*timestamp) < HASH_RETENTION_PERIOD);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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
    
    let producer_handle = tokio::spawn(producer_task(
        sender,
        turn_tracker.clone(),
    ));
    
    let consumer_handle = tokio::spawn(consumer_task(
        receiver,
        semaphore,
        active_processes,
        turn_tracker.clone(),
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

async fn producer_task(
    sender: mpsc::Sender<GameTurn>,
    turn_tracker: TurnTracker,
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
            
            let turn = GameTurn {
                game_state: game_state.to_string(),
                hash,
            };

            if turn_tracker.tracked(&turn) {
                continue;
            }

            turn_tracker.processing(&turn);

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

    let mut child = Command::new("your-command")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to spawn command");

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin.write_all(turn.game_state.as_bytes())
            .expect("Failed to write to stdin");
    }

    let status = child.wait()
        .expect("Failed to wait for child process");

    println!("Process exited with status: {}", status);
    
    turn_tracker.processed(&turn);
}

async fn cleanup_processes(active_processes: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>) {
    let mut processes = active_processes.lock().await;
    processes.retain(|handle| !handle.is_finished());
}
