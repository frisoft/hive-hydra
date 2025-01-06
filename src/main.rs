use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex, Semaphore};
use tracing::{info, warn, error, debug};

mod turn_tracker;
use turn_tracker::{TurnTracker, TurnTracking};
mod ai;
mod hivegame_bot_api;
use hivegame_bot_api::{HiveGameApi, GameHash};
mod config;
use config::{BotConfig, Config};
mod cli;
mod logging;

struct GameTurn {
    game_string: String,
    hash: u64,
    bot: BotConfig,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Initialize logging first
    logging::setup_logging()?;
    info!("Starting Hive Hydra");

    // Parse command line arguments
    let cli = cli::Cli::parse();
    debug!("CLI arguments parsed");

    // Load configuration from specified file
    let config = Config::load_from(cli.config)?;
    info!("Configuration loaded, max concurrent processes: {}", config.max_concurrent_processes);

    let (sender, receiver) = mpsc::channel(config.queue_capacity);
    let receiver = Arc::new(Mutex::new(receiver));
    let semaphore = Arc::new(Semaphore::new(config.max_concurrent_processes));
    let active_processes = Arc::new(Mutex::new(Vec::new()));
    let turn_tracker = TurnTracker::new();

    info!("Initialized channel with capacity: {}", config.queue_capacity);

    let cleanup_tracker = turn_tracker.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            cleanup_tracker.cleanup().await;
            debug!("Cleanup cycle completed");
        }
    });

    // Spawn a producer task for each bot
    let mut producer_handles = Vec::new();
    info!("Starting producer tasks for {} bots", config.bots.len());
    
    for bot in config.bots {
        info!("Spawning producer task for bot: {}", bot.name);
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
    info!("Consumer task started");

    // Wait for all producers and the consumer
    for handle in producer_handles {
        if let Err(e) = handle.await? {
            error!("Producer task error: {}", e);
        }
    }
    if let Err(e) = consumer_handle.await? {
        error!("Consumer task error: {}", e);
    }

    Ok(())
}


async fn producer_task(
    sender: mpsc::Sender<GameTurn>,
    turn_tracker: TurnTracker,
    base_url: String,
    bot: BotConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let api = HiveGameApi::new(base_url);
    info!("Producer task started for bot: {}", bot.name);

    loop {
        match api.fake_get_games(&bot.uri, &bot.api_key).await {
            Ok(game_strings) => {
                debug!("Retrieved {} games for bot {}", game_strings.len(), bot.name);
                for game in game_strings {
                    let hash = game.calculate_hash();

                    if turn_tracker.tracked(hash).await {
                        debug!("Game {} already tracked for bot {}", hash, bot.name);
                        continue;
                    }

                    let turn = GameTurn {
                        game_string: format!("{};{};{};{};{}", 
                            game.game_id,
                            game.game_type,
                            game.game_status,
                            game.player_turn,
                            game.moves),
                        hash,
                        bot: bot.clone(),
                    };

                    turn_tracker.processing(hash).await;
                    debug!("Processing game {} for bot {}", hash, bot.name);

                    if sender.send(turn).await.is_err() {
                        error!("Failed to send turn to queue for bot {}", bot.name);
                        continue;
                    }
                }
            }
            Err(e) => error!("Failed to fetch games for bot {}: {}", bot.name, e),
        }

        debug!("Starting new cycle for bot {}", bot.name);
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn consumer_task(
    receiver: Arc<Mutex<mpsc::Receiver<GameTurn>>>,
    semaphore: Arc<Semaphore>,
    active_processes: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    turn_tracker: TurnTracker,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!("Consumer task started");
    
    loop {
        let mut rx = receiver.lock().await;
        if let Some(turn) = rx.recv().await {
            drop(rx);
            debug!("Received turn for bot {}", turn.bot.name);

            let handle = tokio::spawn(process_turn(
                turn,
                semaphore.clone(),
                turn_tracker.clone()
            ));

            active_processes.lock().await.push(handle);
            cleanup_processes(active_processes.clone()).await;
        }
    }
}

async fn process_turn(turn: GameTurn, semaphore: Arc<Semaphore>, turn_tracker: TurnTracker) {
    let _permit = match semaphore.acquire().await {
        Ok(permit) => permit,
        Err(e) => {
            error!("Failed to acquire semaphore for bot {}: {}", turn.bot.name, e);
            turn_tracker.processed(turn.hash).await;
            return;
        }
    };

    debug!("Processing turn for bot {} with hash {}", turn.bot.name, turn.hash);

    let child = match ai::spawn_process(&turn.bot.ai_command, &turn.bot.name) {
        Ok(child) => child,
        Err(e) => {
            error!("Failed to spawn AI process for bot {}: {}", turn.bot.name, e);
            turn_tracker.processed(turn.hash).await;
            return;
        }
    };

    match ai::run_commands(child, &turn.game_string, &turn.bot.bestmove_command_args).await {
        Ok(bestmove) => {
            info!("Bot '{}' bestmove: '{}'", turn.bot.name, bestmove);
            // Here you can handle the bestmove (e.g., send it to the server)
        }
        Err(e) => {
            error!("Error running AI commands for bot '{}': '{}'", turn.bot.name, e);
        }
    }

    turn_tracker.processed(turn.hash).await;
    debug!("Turn processed for bot {} with hash {}", turn.bot.name, turn.hash);
}

async fn cleanup_processes(active_processes: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>) {
    let mut processes = active_processes.lock().await;
    let initial_count = processes.len();
    processes.retain(|handle| !handle.is_finished());
    let removed = initial_count - processes.len();
    if removed > 0 {
        debug!("Cleaned up {} finished processes", removed);
    }
}
