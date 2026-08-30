#[cfg(test)]
mod tests {
    use crate::core::tasks::manager::{AddTaskOutcome, TaskManager};
    use crate::core::tasks::models::TaskNode;

    #[test]
    fn test_add_task() {
        let mut manager = TaskManager::new();
        let task = TaskNode::new("Test Task".to_string());
        let id = task.id.clone();

        assert!(manager.add_task(task).is_ok());
        assert!(manager.get_task(&id).is_some());
        assert_eq!(manager.graph.root_ids.len(), 1);
    }

    #[test]
    fn add_task_dedup_reuses_active_task_with_same_title_and_parent() {
        let mut manager = TaskManager::new();

        let first = TaskNode::new("Fix tool loop".to_string());
        let first_id = first.id.clone();
        assert_eq!(
            manager.add_task_dedup(first).unwrap(),
            AddTaskOutcome::Added(first_id.clone())
        );

        let second = TaskNode::new("Fix tool loop".to_string());
        assert_eq!(
            manager.add_task_dedup(second).unwrap(),
            AddTaskOutcome::Existing(first_id)
        );
        assert_eq!(manager.graph.nodes.len(), 1);
        assert_eq!(manager.graph.root_ids.len(), 1);
    }

    #[test]
    fn load_from_file_repairs_duplicate_roots_and_missing_children() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join(".star").join("tasks.json");
        let mut manager = TaskManager::new();

        let parent = TaskNode::new("Parent".to_string());
        let parent_id = parent.id.clone();
        manager.add_task(parent).unwrap();

        let mut child = TaskNode::new("Child".to_string());
        child.parent_id = Some(parent_id.clone());
        let child_id = child.id.clone();
        manager.graph.nodes.insert(child_id.clone(), child);
        manager.graph.root_ids.push(parent_id.clone());
        manager.graph.root_ids.push(parent_id.clone());

        manager.save_to_file(&path).unwrap();
        let loaded = TaskManager::load_from_file(&path).unwrap();

        assert_eq!(loaded.graph.root_ids, vec![parent_id.clone()]);
        assert_eq!(
            loaded.graph.nodes.get(&parent_id).unwrap().children,
            vec![child_id]
        );
    }

    #[test]
    fn test_delete_task_cascade() {
        let mut manager = TaskManager::new();

        // Create root
        let root = TaskNode::new("Root".to_string());
        let root_id = root.id.clone();
        manager.add_task(root).unwrap();

        // Create child
        let mut child = TaskNode::new("Child".to_string());
        child.parent_id = Some(root_id.clone());
        let child_id = child.id.clone();
        manager.add_task(child).unwrap();

        // Delete root
        manager.delete_task(&root_id).unwrap();

        // Both should be gone
        assert!(manager.get_task(&root_id).is_none());
        assert!(manager.get_task(&child_id).is_none());
        assert!(manager.graph.root_ids.is_empty());
    }

    #[test]
    fn test_move_task_reorder() {
        let mut manager = TaskManager::new();

        let t1 = TaskNode::new("Task 1".to_string());
        let t1_id = t1.id.clone();
        manager.add_task(t1).unwrap();

        let t2 = TaskNode::new("Task 2".to_string());
        let t2_id = t2.id.clone();
        manager.add_task(t2).unwrap();

        let t3 = TaskNode::new("Task 3".to_string());
        let t3_id = t3.id.clone();
        manager.add_task(t3).unwrap();

        // Initial order: t1, t2, t3
        assert_eq!(
            manager.graph.root_ids,
            vec![t1_id.clone(), t2_id.clone(), t3_id.clone()]
        );

        // Move t3 to after t1 (Result: t1, t3, t2)
        manager
            .move_task(&t3_id, None, Some(t1_id.clone()))
            .unwrap();
        assert_eq!(
            manager.graph.root_ids,
            vec![t1_id.clone(), t3_id.clone(), t2_id.clone()]
        );

        // Move t1 to end (Result: t3, t2, t1)
        manager
            .move_task(&t1_id, None, Some(t2_id.clone()))
            .unwrap();
        assert_eq!(
            manager.graph.root_ids,
            vec![t3_id.clone(), t2_id.clone(), t1_id.clone()]
        );

        // Move t2 to start (Result: t2, t3, t1)
        // To move to start, we can't use "after_id".
        // Wait, move_task(id, None, None) means insert at 0?
        // Let's check implementation of move_task.
        // If after_id is None, it inserts at 0.
        manager.move_task(&t2_id, None, None).unwrap();
        assert_eq!(
            manager.graph.root_ids,
            vec![t2_id.clone(), t3_id.clone(), t1_id.clone()]
        );
    }

    #[test]
    fn test_move_task_hierarchy() {
        let mut manager = TaskManager::new();

        let parent = TaskNode::new("Parent".to_string());
        let p_id = parent.id.clone();
        manager.add_task(parent).unwrap();

        let child = TaskNode::new("Child".to_string());
        let c_id = child.id.clone();
        manager.add_task(child).unwrap();

        // Move child into parent
        manager.move_task(&c_id, Some(p_id.clone()), None).unwrap();

        let p = manager.get_task(&p_id).unwrap();
        assert!(p.children.contains(&c_id));
        assert!(!manager.graph.root_ids.contains(&c_id));

        let c = manager.get_task(&c_id).unwrap();
        assert_eq!(c.parent_id, Some(p_id.clone()));
    }

    #[test]
    fn test_cycle_detection() {
        let mut manager = TaskManager::new();

        let t1 = TaskNode::new("T1".to_string());
        let t1_id = t1.id.clone();
        manager.add_task(t1).unwrap();

        let mut t2 = TaskNode::new("T2".to_string());
        let t2_id = t2.id.clone();
        // t2 depends on t1
        t2.dependencies.push(t1_id.clone());
        manager.add_task(t2).unwrap();

        // No cycle yet
        assert!(!manager.detect_cycles());

        // Add cycle: t1 depends on t2
        let mut t1_update = manager.get_task(&t1_id).unwrap().clone();
        t1_update.dependencies.push(t2_id.clone());
        // We use update_task which doesn't check cycles automatically (as per implementation note)
        // But detect_cycles() should catch it.
        manager.update_task(t1_update).unwrap();

        assert!(manager.detect_cycles());
    }

    #[test]
    fn test_execution_plan() {
        let mut manager = TaskManager::new();

        // A -> B -> C
        //      |
        //      v
        //      D

        let a = TaskNode::new("A".to_string());
        let a_id = a.id.clone();
        manager.add_task(a).unwrap();

        let mut b = TaskNode::new("B".to_string());
        b.dependencies.push(a_id.clone());
        let b_id = b.id.clone();
        manager.add_task(b).unwrap();

        let mut c = TaskNode::new("C".to_string());
        c.dependencies.push(b_id.clone());
        let c_id = c.id.clone();
        manager.add_task(c).unwrap();

        let mut d = TaskNode::new("D".to_string());
        d.dependencies.push(b_id.clone());
        let d_id = d.id.clone();
        manager.add_task(d).unwrap();

        let plan = manager.get_execution_plan();

        assert_eq!(plan.len(), 3);
        assert_eq!(plan[0][0].id, a_id);
        assert_eq!(plan[1][0].id, b_id);

        // Layer 3 can have C and D in any order
        let layer3_ids: Vec<String> = plan[2].iter().map(|t| t.id.clone()).collect();
        assert!(layer3_ids.contains(&c_id));
        assert!(layer3_ids.contains(&d_id));
    }
}
