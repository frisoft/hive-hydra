use actix_web::{web, App, HttpServer};
use log::{error, info};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::io::{Write, BufRead, BufReader};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{interval, Duration};

#[derive(Debug, Deserialize, Clone)]
struct GameState {
    game_id: String,
    state: String,
    // Add other relevant game state fields
}

#[derive(Debug, Serialize)]
struct GameMove {
    game_id: String,
    movement: String,
}

struct AiProcess {
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
}

// Track active games and their AI processes
#[derive(Debug)]
struct AppState {
    active_games: Arc<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>>,
}

async fn start_ai_process() -> std::io::Result<AiProcess> {
    let mut child = Command::new("./ai_program")  // Replace with your AI program path
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    let stdin = child.stdin.take().expect("Failed to get stdin");
    let stdout = BufReader::new(child.stdout.take().expect("Failed to get stdout"));

    Ok(AiProcess { stdin, stdout })
}

async fn get_ai_move(ai_process: &mut AiProcess, game_state: &str) -> std::io::Result<String> {
    writeln!(ai_process.stdin, "{}", game_state)?;
    let mut response = String::new();
    ai_process.stdout.read_line(&mut response)?;
    Ok(response.trim().to_string())
}

async fn handle_game(game: GameState, client: reqwest::Client) {
    info!("Starting AI process for game: {}", game.game_id);
    
    match start_ai_process().await {
        Ok(mut ai_process) => {
            loop {
                // Get AI move for current game state
                match get_ai_move(&mut ai_process, &game.state).await {
                    Ok(movement) => {
                        let game_move = GameMove {
                            game_id: game.game_id.clone(),
                            movement,
                        };
                        
                        match client.post("http://remote-server/move")
                            .json(&game_move)
                            .send()
                            .await 
                        {
                            Ok(_) => info!("Move sent for game {}", game.game_id),
                            Err(e) => {
                                error!("Failed to send move for game {}: {}", game.game_id, e);
                                break;
                            }
                        }
                        
                        // Wait before next move
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                    Err(e) => {
                        error!("AI process error for game {}: {}", game.game_id, e);
                        break;
                    }
                }
            }
        }
        Err(e) => error!("Failed to start AI process for game {}: {}", game.game_id, e),
    }
}

// Background job that polls for games and manages AI moves
async fn game_monitor(state: web::Data<AppState>) {
    let client = reqwest::Client::new();
    let mut interval = interval(Duration::from_secs(5));
    
    loop {
        interval.tick().await;
        
        // Clean up finished game tasks
        {
            let mut active_games = state.active_games.lock().await;
            active_games.retain(|game_id, task| {
                if task.is_finished() {
                    info!("Removing finished game task: {}", game_id);
                    false
                } else {
                    true
                }
            });
        }
        
        // Fetch new games from remote server
        match client.get("http://remote-server/games").send().await {
            Ok(response) => {
                match response.json::<Vec<GameState>>().await {
                    Ok(games) => {
                        for game in games {
                            let mut active_games = state.active_games.lock().await;
                            
                            // Skip if game is already being processed
                            if active_games.contains_key(&game.game_id) {
                                continue;
                            }

                            // Clone necessary data before moving into the task
                            let game_id = game.game_id.clone();
                            let game_client = client.clone();
                            let game_clone = game.clone();
                            let handle = tokio::spawn(async move {
                                handle_game(game_clone, game_client).await;
                            });
                            
                            active_games.insert(game_id, handle); 
                            info!("Started new game task: {}", game.game_id);
                        }
                    }
                    Err(e) => error!("Failed to parse games response: {}", e),
                }
            }
            Err(e) => error!("Failed to fetch games: {}", e),
        }
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Initialize logger
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));
    
    // Initialize shared state
    let state = web::Data::new(AppState {
        active_games: Arc::new(Mutex::new(HashMap::new())),
    });
    
    // Spawn background job
    let job_state = state.clone();
    tokio::spawn(async move {
        game_monitor(job_state).await;
    });
    
    info!("Starting server at http://127.0.0.1:8080");
    
    // Start HTTP server
    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            // Add your endpoints here if needed
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}
