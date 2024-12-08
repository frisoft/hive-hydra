use std::process::{Command, Stdio, Child};
use std::io::Write;

#[derive(Debug)]
pub enum AiError {
    ProcessError(std::io::Error),
    ParseError(ParseError),
    OutputError(std::string::FromUtf8Error),
}

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AiError::ProcessError(e) => write!(f, "Process error: {}", e),
            AiError::ParseError(e) => write!(f, "Parse error: {}", e),
            AiError::OutputError(e) => write!(f, "Output encoding error: {}", e),
        }
    }
}

impl std::error::Error for AiError {}

impl From<std::io::Error> for AiError {
    fn from(error: std::io::Error) -> Self {
        AiError::ProcessError(error)
    }
}

impl From<ParseError> for AiError {
    fn from(error: ParseError) -> Self {
        AiError::ParseError(error)
    }
}

impl From<std::string::FromUtf8Error> for AiError {
    fn from(error: std::string::FromUtf8Error) -> Self {
        AiError::OutputError(error)
    }
}

#[derive(Debug)]
pub enum ParseError {
    NoOutput,
    MissingBestMove(String),
    MissingOk(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::NoOutput => write!(f, "No output received from AI"),
            ParseError::MissingBestMove(msg) => write!(f, "No bestmove found in output: {}", msg),
            ParseError::MissingOk(msg) => write!(f, "Missing 'ok' confirmation in output: {}", msg),
        }
    }
}

impl std::error::Error for ParseError {}

pub fn spawn_process(command: &str, name: &str) -> std::io::Result<Child> {
    println!("Starting AI '{}' for '{}'...", command, name);
    
    let command_parts: Vec<&str> = command.split_whitespace().collect();
    
    if command_parts.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("Error: Empty AI command for {}, command {}", name, command)
        ));
    }

    let program = command_parts[0];
    let args = &command_parts[1..];
    
    Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

pub fn parse_ai_output(output: &str) -> Result<String, ParseError> {
    if output.is_empty() {
        return Err(ParseError::NoOutput);
    }

    let lines: Vec<&str> = output
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect();

    let first_ok_pos = lines.iter().position(|&line| line == "ok")
        .ok_or(ParseError::MissingOk(output.to_string()))?;
    let remaining_lines = &lines[first_ok_pos + 1..];
    let second_ok_pos = remaining_lines.iter().position(|&line| line == "ok")
        .ok_or(ParseError::MissingOk(output.to_string()))?;

    let remaining_lines = &remaining_lines[second_ok_pos + 1..];
    let bestmove_pos = remaining_lines.iter()
        .position(|&line| line != "ok")
        .ok_or(ParseError::MissingBestMove(output.to_string()))?;

    let bestmove = remaining_lines[bestmove_pos];

    if !remaining_lines[bestmove_pos + 1..].contains(&"ok") {
        return Err(ParseError::MissingOk(output.to_string()));
    }

    Ok(bestmove.to_string())
}

pub async fn run_commands(mut child: Child, game_string: &str, bestmove_args: &str) -> Result<String, AiError> {
    let stdin = child.stdin.take()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "Failed to open stdin"))?;

    let mut stdin = stdin;
    
    // Send newgame command
    let newgame_command = format!("newgame {}\n", game_string);
    stdin.write_all(newgame_command.as_bytes())?;

    // Send bestmove command
    let bestmove_command = format!("bestmove {}\n", bestmove_args);
    stdin.write_all(bestmove_command.as_bytes())?;

    // We're done with stdin, drop it explicitly to signal EOF to the child process
    drop(stdin);
    
    // Read output
    let output = child.wait_with_output()?;
    let stdout = String::from_utf8(output.stdout)?;
    
    Ok(parse_ai_output(&stdout)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_output() {
        let output = r#"
            id nokamute cargo-1.0.0
            Mosquito;Ladybug;Pillbug
            ok
            Base;InProgress;White[3];wS1
            ok
            bG1 -wS1
            ok
        "#;
        assert_eq!(parse_ai_output(output).unwrap(), "bG1 -wS1");
    }

    #[test]
    fn test_empty_output() {
        let output = "";
        assert!(matches!(parse_ai_output(output), Err(ParseError::NoOutput)));
    }

    #[test]
    fn test_missing_ok() {
        let output = r#"
            id nokamute cargo-1.0.0
            Mosquito;Ladybug;Pillbug
            ok
            Base;InProgress;White[3];wS1
            bG1 -wS1
        "#;
        assert!(matches!(parse_ai_output(output), Err(ParseError::MissingOk(_))));
    }

    #[test]
    fn test_missing_bestmove() {
        let output = r#"
            id nokamute cargo-1.0.0
            Mosquito;Ladybug;Pillbug
            ok
            Base;InProgress;White[3];wS1
            ok
            ok
        "#;
        assert!(matches!(parse_ai_output(output), Err(ParseError::MissingBestMove(_))));
    }
}