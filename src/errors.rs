use std::io;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum OreMinerError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Failed to parse PID: {0}")]
    PidParse(#[from] std::num::ParseIntError),

    #[error("Daemon error: {0}")]
    Daemon(String),

    #[error("CloudWatch error: {0}")]
    CloudWatch(String),

    #[error("Command execution error: {0}")]
    CommandExecution(String),

    #[error("Binary not found: {0}. Details: {1}")]
    BinaryNotFound(String, String),

    #[error("Environment variable not set: {0}")]
    EnvVar(String),

    #[error("Parse error: {0}")]
    ParseError(String),
}

/**
 * Clone is implemented to allow for easy cloning of errors.
 * PartialEq is implemented to allow for easy comparison of errors.
 *
 * The default implementations of each do not work with `io::Error`
 */
impl Clone for OreMinerError {
    fn clone(&self) -> Self {
        match self {
            Self::Io(e) => Self::Io(io::Error::new(e.kind(), e.to_string())),
            Self::PidParse(e) => Self::PidParse(e.clone()),
            Self::Daemon(s) => Self::Daemon(s.clone()),
            Self::CloudWatch(s) => Self::CloudWatch(s.clone()),
            Self::CommandExecution(s) => Self::CommandExecution(s.clone()),
            Self::BinaryNotFound(s1, s2) => Self::BinaryNotFound(s1.clone(), s2.clone()),
            Self::EnvVar(s) => Self::EnvVar(s.clone()),
            Self::ParseError(s) => Self::ParseError(s.clone()),
        }
    }
}

impl PartialEq for OreMinerError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Io(e1), Self::Io(e2)) => {
                e1.kind() == e2.kind() && e1.to_string() == e2.to_string()
            }
            (Self::PidParse(e1), Self::PidParse(e2)) => e1 == e2,
            (Self::Daemon(s1), Self::Daemon(s2)) => s1 == s2,
            (Self::CloudWatch(s1), Self::CloudWatch(s2)) => s1 == s2,
            (Self::CommandExecution(s1), Self::CommandExecution(s2)) => s1 == s2,
            (Self::BinaryNotFound(s1, s2), Self::BinaryNotFound(s3, s4)) => s1 == s3 && s2 == s4,
            (Self::EnvVar(s1), Self::EnvVar(s2)) => s1 == s2,
            (Self::ParseError(s1), Self::ParseError(s2)) => s1 == s2,
            _ => core::mem::discriminant(self) == core::mem::discriminant(other),
        }
    }
}

pub type Result<T> = std::result::Result<T, OreMinerError>;
