//! Unified command representation (CommandAST).

/// Source protocol of the command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CommandSource {
    #[default]
    Cli,
    JsonRpc,
    Grpc,
    Internal,
}

/// Response format for command output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResponseFormat {
    #[default]
    Default,
    Text,
    Api,
    Json,
    Grpc,
}

/// A single command argument.
#[derive(Debug, Clone)]
pub struct Argument {
    pub key: String,
    pub value: Option<String>,
}

/// Parsed command ready for execution.
#[derive(Debug, Clone, Default)]
pub struct CommandAst {
    pub command_name: String,
    pub arguments: Vec<Argument>,
    pub session_id: String,
    pub source: CommandSource,
    pub response_format: ResponseFormat,
    /// Console input line as received (C++ `ua->cmd`); empty when the
    /// command did not come from a text line.
    pub raw_line: String,
}
