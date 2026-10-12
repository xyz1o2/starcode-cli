//! Deferred manager tests

use starcode_cli::core::deferred::{DeferredError, DeferredManager};

#[derive(Debug, PartialEq)]
enum TestError {
    Rejected,
}

#[tokio::test]
async fn test_deferred_create_and_resolve() {
    let manager = DeferredManager::<String, TestError>::new();
    let id = "test-id".to_string();

    let rx = manager.create(id.clone()).await;

    assert!(manager.has_pending(&id).await);
    assert_eq!(manager.pending_count().await, 1);

    manager.resolve(&id, "success".to_string()).await.unwrap();

    let result = rx.await.unwrap();
    assert_eq!(result, Ok("success".to_string()));

    assert!(!manager.has_pending(&id).await);
    assert_eq!(manager.pending_count().await, 0);
}

#[tokio::test]
async fn test_deferred_create_and_reject() {
    let manager = DeferredManager::<String, TestError>::new();
    let id = "test-id".to_string();

    let rx = manager.create(id.clone()).await;

    manager.reject(&id, TestError::Rejected).await.unwrap();

    let result = rx.await.unwrap();
    assert_eq!(result, Err(TestError::Rejected));
}

#[tokio::test]
async fn test_deferred_not_found() {
    let manager = DeferredManager::<String, TestError>::new();

    let result = manager.resolve("nonexistent", "value".to_string()).await;
    assert!(matches!(result, Err(DeferredError::NotFound)));

    let result = manager.reject("nonexistent", TestError::Rejected).await;
    assert!(matches!(result, Err(DeferredError::NotFound)));
}

#[tokio::test]
async fn test_deferred_pending_ids() {
    let manager = DeferredManager::<String, TestError>::new();

    let _rx1 = manager.create("id-1".to_string()).await;
    let _rx2 = manager.create("id-2".to_string()).await;

    let ids = manager.pending_ids().await;
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&"id-1".to_string()));
    assert!(ids.contains(&"id-2".to_string()));
}

#[tokio::test]
async fn test_deferred_multiple_resolves() {
    let manager = DeferredManager::<String, TestError>::new();
    let id = "test-id".to_string();

    let rx = manager.create(id.clone()).await;

    // First resolve should succeed
    assert!(manager.resolve(&id, "first".to_string()).await.is_ok());

    // Second resolve should fail (already resolved)
    assert!(manager.resolve(&id, "second".to_string()).await.is_err());

    let result = rx.await.unwrap();
    assert_eq!(result, Ok("first".to_string()));
}
