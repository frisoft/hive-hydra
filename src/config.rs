use config::{Config as ConfigBuilder, ConfigError, Environment, File};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub max_concurrent_processes: usize,
    pub queue_capacity: usize,
    pub base_url: String,
    pub bots: Vec<BotConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotConfig {
    pub name: String,
    pub uri: String,
    pub ai_command: String,
    pub bestmove_command_args: String,
    pub api_key: String,
}

impl Config {
    pub fn load_from<P: AsRef<Path>>(config_path: P) -> Result<Self, ConfigError> {
        let settings = ConfigBuilder::builder()
            // 1. Default values
            .set_default("max_concurrent_processes", 5)?
            .set_default("queue_capacity", 1000)?
            .set_default("base_url", "https://hivegame.com")?
            // 2. Config file
            .add_source(File::from(config_path.as_ref()))
            // 3. Environment variables
            .add_source(Environment::with_prefix("HIVE_HYDRA"))
            .build()?;

        // Load the main configuration
        let mut config: Config = settings.try_deserialize()?;

        // Process bot-specific environment variables
        let env_vars: HashMap<String, String> = std::env::vars().collect();
        for bot in &mut config.bots {
            let prefix = format!("HIVE_HYDRA_BOT_{}_", bot.name.to_uppercase());

            if let Some(value) = env_vars.get(&format!("{}API_KEY", prefix)) {
                bot.api_key = value.clone();
            }
        }

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_env_override_with_custom_config() {
        // Clear existing env vars
        env::remove_var("HIVE_HYDRA_MAX_CONCURRENT_PROCESSES");
        env::remove_var("HIVE_HYDRA_BOT_TESTBOT1_API_KEY");

        // Set test env vars
        env::set_var("HIVE_HYDRA_MAX_CONCURRENT_PROCESSES", "10");
        env::set_var("HIVE_HYDRA_BOT_TESTBOT1_API_KEY", "test_key_1");

        // Create test config file
        let config_content = r#"
max_concurrent_processes: 5
queue_capacity: 1000
base_url: "https://hivegame.com"
bots:
  - name: testbot1
    uri: /games/testbot1
    ai_command: test_command
    bestmove_command_args: depth 1
    api_key: default_key1
"#;
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("config.yaml");
        fs::write(&file_path, config_content).unwrap();

        // Load config and test
        let config = Config::load_from(&file_path).unwrap();
        assert_eq!(
            config.max_concurrent_processes, 10,
            "max_concurrent_processes should be overridden by environment variable"
        );
        assert_eq!(
            config.bots[0].api_key, "test_key_1",
            "bot api_key should be overridden by environment variable"
        );

        // Cleanup
        env::remove_var("HIVE_HYDRA_MAX_CONCURRENT_PROCESSES");
        env::remove_var("HIVE_HYDRA_BOT_TESTBOT1_API_KEY");
    }
}