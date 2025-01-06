use reqwest::{Client, Error as ReqwestError};
use std::time::Duration;
use serde::{Deserialize, Serialize};

const API_TIMEOUT: u64 = 10; // 10 seconds timeout for API calls

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Request failed: {0}")]
    RequestError(#[from] ReqwestError),
    #[error("API error: {status_code} - {message}")]
    ApiError {
        status_code: reqwest::StatusCode,
        message: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HiveGame {
    game_id: String,
    time: String,
    opponent_username: String,
    game_type: String, // Base, Base+PLM
    game_status: String, // InProgress, etc.
    player_turn: String, // White[3]
    moves: String // wS1;bG1 -wS1;wA1 wS1/;bG2 /bG 
}

pub struct HiveGameApi {
    client: Client,
    base_url: String,
}

impl HiveGameApi {
    pub fn new(base_url: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(API_TIMEOUT))
            .build()
            .expect("Failed to create HTTP client");

        Self { client, base_url }
    }

    /// Get all active games for a bot
    /// Returns a vector of HiveGame
    pub async fn get_games(&self, uri: &str, api_key: &str) -> Result<Vec<HiveGame>, ApiError> {
        let url = format!("{}{}", self.base_url, uri);

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            return Err(ApiError::ApiError {
                status_code: status,
                message: response.text().await.unwrap_or_default(),
            });
        }

        response.json().await.map_err(|e| ApiError::RequestError(e))
    }

    /// Function to use for manual testing without real connection
    pub async fn fake_get_games(
        &self,
        _uri: &str,
        _api_key: &str,
    ) -> Result<Vec<HiveGame>, ApiError> {
        let game = HiveGame {
            game_id: "123".to_string(),
            time: "20+10".to_string(),
            opponent_username: "player1".to_string(),
            game_type: "Base+PLM".to_string(),
            game_status: "InProgress".to_string(),
            player_turn: "White[3]".to_string(),
            moves: "wS1;bG1 -wS1;wA1 wS1/;bG2 /bG1".to_string(),
        };
        Ok(vec![game])
    }

    /// Send a move to the game
    pub async fn play_move(
        &self,
        game_id: &str,
        move_notation: &str,
        api_key: &str,
    ) -> Result<(), ApiError> {
        let url = format!("{}/games/{}/move", self.base_url, game_id);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&move_notation)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            return Err(ApiError::ApiError {
                status_code: status,
                message: response.text().await.unwrap_or_default(),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_get_games() {
        // Start a mock server
        let mock_server = MockServer::start().await;

        // Create mock response
        Mock::given(method("GET"))
            .and(path("/games/bot1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(vec!["Base;InProgress;White[3];wS1;bG1"]),
            )
            .mount(&mock_server)
            .await;

        let api = HiveGameApi::new(mock_server.uri());
        let games = api.get_games("/games/bot1", "test_key").await.unwrap();

        assert_eq!(games.len(), 1);
        assert!(games[0].contains("Base;InProgress"));
    }

    #[tokio::test]
    async fn test_play_move() {
        // Start a mock server
        let mock_server = MockServer::start().await;

        // Create mock response
        Mock::given(method("POST"))
            .and(path("/games/123/move"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&mock_server)
            .await;

        let api = HiveGameApi::new(mock_server.uri());
        let result = api.play_move("123", "wS1", "test_key").await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_error_handling() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/games/error"))
            .respond_with(ResponseTemplate::new(404).set_body_string("Not found"))
            .mount(&mock_server)
            .await;

        let api = HiveGameApi::new(mock_server.uri());
        let result = api.get_games("/games/error", "test_key").await;

        assert!(matches!(result,
            Err(ApiError::ApiError {
                status_code,
                message
            }) if status_code == 404 && message == "Not found"
        ));
    }
}
