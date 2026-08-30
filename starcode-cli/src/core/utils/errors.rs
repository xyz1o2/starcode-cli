use std::fmt;

#[derive(Debug)]
pub struct FatalError {
    pub message: String,
    pub exit_code: i32,
}

impl FatalError {
    pub fn new(message: String, exit_code: i32) -> Self {
        Self { message, exit_code }
    }
}

impl fmt::Display for FatalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for FatalError {}

#[derive(Debug)]
pub struct FatalAuthenticationError {
    pub message: String,
}

impl FatalAuthenticationError {
    pub fn new(message: String) -> Self {
        Self { message }
    }
}

impl fmt::Display for FatalAuthenticationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for FatalAuthenticationError {}

#[derive(Debug)]
pub struct FatalInputError {
    pub message: String,
}

impl FatalInputError {
    pub fn new(message: String) -> Self {
        Self { message }
    }
}

impl fmt::Display for FatalInputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for FatalInputError {}

#[derive(Debug)]
pub struct FatalSandboxError {
    pub message: String,
}

impl FatalSandboxError {
    pub fn new(message: String) -> Self {
        Self { message }
    }
}

impl fmt::Display for FatalSandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for FatalSandboxError {}

#[derive(Debug)]
pub struct FatalConfigError {
    pub message: String,
}

impl FatalConfigError {
    pub fn new(message: String) -> Self {
        Self { message }
    }
}

impl fmt::Display for FatalConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for FatalConfigError {}

#[derive(Debug)]
pub struct FatalTurnLimitedError {
    pub message: String,
}

impl FatalTurnLimitedError {
    pub fn new(message: String) -> Self {
        Self { message }
    }
}

impl fmt::Display for FatalTurnLimitedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for FatalTurnLimitedError {}

#[derive(Debug)]
pub struct FatalToolExecutionError {
    pub message: String,
}

impl FatalToolExecutionError {
    pub fn new(message: String) -> Self {
        Self { message }
    }
}

impl fmt::Display for FatalToolExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for FatalToolExecutionError {}

#[derive(Debug)]
pub struct FatalCancellationError {
    pub message: String,
}

impl FatalCancellationError {
    pub fn new(message: String) -> Self {
        Self { message }
    }
}

impl fmt::Display for FatalCancellationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for FatalCancellationError {}

#[derive(Debug)]
pub struct CanceledError {
    pub message: String,
}

impl CanceledError {
    pub fn new(message: String) -> Self {
        Self { message }
    }
}

impl Default for CanceledError {
    fn default() -> Self {
        Self::new("The operation was canceled.".to_string())
    }
}

impl fmt::Display for CanceledError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for CanceledError {}

#[derive(Debug)]
pub struct ForbiddenError {
    pub message: String,
}

impl ForbiddenError {
    pub fn new(message: String) -> Self {
        Self { message }
    }
}

impl fmt::Display for ForbiddenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ForbiddenError {}

#[derive(Debug)]
pub struct UnauthorizedError {
    pub message: String,
}

impl UnauthorizedError {
    pub fn new(message: String) -> Self {
        Self { message }
    }
}

impl fmt::Display for UnauthorizedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for UnauthorizedError {}

#[derive(Debug)]
pub struct BadRequestError {
    pub message: String,
}

impl BadRequestError {
    pub fn new(message: String) -> Self {
        Self { message }
    }
}

impl fmt::Display for BadRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for BadRequestError {}

pub fn get_error_message(error: &dyn std::error::Error) -> String {
    error.to_string()
}

pub fn is_authentication_error(error: &dyn std::error::Error) -> bool {
    error.to_string().contains("401")
        || error.to_string().contains("unauthorized")
        || error.to_string().contains("authentication")
}
