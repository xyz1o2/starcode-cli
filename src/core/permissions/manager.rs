//! Permission manager for handling permission requests

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::core::deferred::DeferredManager;
use crate::core::events::{EventBus, PermissionReply, PermissionRequest, SessionEvent};

use super::evaluator::PermissionEvaluator;
use super::rules::{PermissionEffect, PermissionRule};
use super::PermissionError;

/// Permission manager
pub struct PermissionManager {
    deferred: DeferredManager<(), PermissionError>,
    events: EventBus,
    evaluator: Arc<Mutex<PermissionEvaluator>>,
    pending: Arc<Mutex<HashMap<String, PermissionRequest>>>,
}

impl PermissionManager {
    /// Create a new permission manager
    pub fn new(events: EventBus) -> Self {
        Self {
            deferred: DeferredManager::new(),
            events,
            evaluator: Arc::new(Mutex::new(PermissionEvaluator::new())),
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Create with initial rules
    pub fn with_rules(events: EventBus, rules: Vec<PermissionRule>) -> Self {
        Self {
            deferred: DeferredManager::new(),
            events,
            evaluator: Arc::new(Mutex::new(PermissionEvaluator::with_rules(rules))),
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Add a permission rule
    pub async fn add_rule(&self, rule: PermissionRule) {
        let mut evaluator = self.evaluator.lock().await;
        evaluator.add_rule(rule);
    }

    /// Set saved rules for a project
    pub async fn set_saved_rules(&self, project_id: &str, rules: Vec<PermissionRule>) {
        let mut evaluator = self.evaluator.lock().await;
        evaluator.set_saved_rules(project_id, rules);
    }

    /// Request permission for an action
    pub async fn request(
        &self,
        request: PermissionRequest,
    ) -> Result<(), PermissionError> {
        // Evaluate permission
        let effect = {
            let evaluator = self.evaluator.lock().await;
            evaluator.evaluate(
                &request.action,
                request.resources.first().map(|s| s.as_str()).unwrap_or("*"),
                None, // TODO: Pass project ID
            )
        };

        match effect {
            PermissionEffect::Allow => Ok(()),
            PermissionEffect::Deny => Err(PermissionError::Denied),
            PermissionEffect::Ask => {
                // Store pending request
                {
                    let mut pending = self.pending.lock().await;
                    pending.insert(request.id.clone(), request.clone());
                }

                // Publish event
                self.events
                    .publish(SessionEvent::PermissionRequested {
                        session_id: request.session_id.clone(),
                        request_id: request.id.clone(),
                        action: request.action.clone(),
                        resources: request.resources.clone(),
                        timestamp: chrono::Utc::now(),
                    })
                    .ok();

                // Create deferred and wait for response
                let rx = self.deferred.create(request.id.clone()).await;

                // Wait for response
                match rx.await {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(e)) => Err(e),
                    Err(_) => Err(PermissionError::Rejected),
                }
            }
        }
    }

    /// Reply to a permission request
    pub async fn reply(
        &self,
        request_id: &str,
        reply: PermissionReply,
    ) -> Result<(), crate::core::deferred::DeferredError> {
        // Get pending request
        let request = {
            let mut pending = self.pending.lock().await;
            pending.remove(request_id)
        };

        if let Some(request) = request {
            // Publish event
            self.events
                .publish(SessionEvent::PermissionReplied {
                    session_id: request.session_id.clone(),
                    request_id: request_id.to_string(),
                    reply: reply.clone(),
                    timestamp: chrono::Utc::now(),
                })
                .ok();

            // Handle reply
            match reply {
                PermissionReply::Once => {
                    self.deferred.resolve(request_id, ()).await?;
                }
                PermissionReply::Always => {
                    // Save rule if requested
                    if let Some(save) = &request.save {
                        let mut evaluator = self.evaluator.lock().await;
                        for resource in save {
                            evaluator.add_saved_rule(
                                &request.session_id, // TODO: Use project ID
                                PermissionRule::allow(&request.action, resource),
                            );
                        }
                    }
                    self.deferred.resolve(request_id, ()).await?;
                }
                PermissionReply::Reject => {
                    self.deferred
                        .reject(request_id, PermissionError::Rejected)
                        .await?;
                }
            }
        }

        Ok(())
    }

    /// Get pending requests
    pub async fn pending_requests(&self) -> Vec<PermissionRequest> {
        let pending = self.pending.lock().await;
        pending.values().cloned().collect()
    }

    /// Check if a request is pending
    pub async fn is_pending(&self, request_id: &str) -> bool {
        let pending = self.pending.lock().await;
        pending.contains_key(request_id)
    }
}

impl Clone for PermissionManager {
    fn clone(&self) -> Self {
        Self {
            deferred: DeferredManager::new(),
            events: self.events.clone(),
            evaluator: self.evaluator.clone(),
            pending: self.pending.clone(),
        }
    }
}
