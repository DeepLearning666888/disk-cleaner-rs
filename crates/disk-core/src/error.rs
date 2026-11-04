//! 统一错误类型：把扫描、删除与 IO 错误收敛到 [`CoreError`]。
//!
//! 各子模块仍返回各自的细分错误类型，调用方需要单一错误出口时用
//! `?` 与 [`From`] 转换即可并入 [`CoreError`]。

use std::fmt;
use std::io;

use crate::delete::DeleteError;
use crate::scan::ScanError;

#[derive(Debug)]
pub enum CoreError {
    Scan(ScanError),
    Delete(DeleteError),
    Io(io::Error),
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoreError::Scan(error) => error.fmt(f),
            CoreError::Delete(error) => error.fmt(f),
            CoreError::Io(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for CoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CoreError::Scan(error) => Some(error),
            CoreError::Delete(error) => Some(error),
            CoreError::Io(error) => Some(error),
        }
    }
}

impl From<ScanError> for CoreError {
    fn from(error: ScanError) -> Self {
        CoreError::Scan(error)
    }
}

impl From<DeleteError> for CoreError {
    fn from(error: DeleteError) -> Self {
        CoreError::Delete(error)
    }
}

impl From<io::Error> for CoreError {
    fn from(error: io::Error) -> Self {
        CoreError::Io(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_scan_error() {
        let error: CoreError = ScanError::Cancelled.into();
        assert_eq!(error.to_string(), "扫描已取消");
        assert!(matches!(error, CoreError::Scan(ScanError::Cancelled)));
    }

    #[test]
    fn converts_delete_error() {
        let error: CoreError = DeleteError::Protected("路径受保护".to_string()).into();
        assert_eq!(error.to_string(), "路径受保护");
        assert!(matches!(error, CoreError::Delete(_)));
    }

    #[test]
    fn converts_io_error_and_keeps_source() {
        let error: CoreError = io::Error::new(io::ErrorKind::PermissionDenied, "拒绝访问").into();
        assert!(error.to_string().contains("拒绝访问"));
        assert!(std::error::Error::source(&error).is_some());
    }
}
