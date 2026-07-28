//! Ergonomic lookup helpers on [`Project`], kept out of `model.rs` so that
//! module stays a plain data shape FRB can mirror in full: these methods
//! return borrowed references (`&Task`, `&Resource`, ...), and a type that's
//! seen both by value (e.g. `Project::tasks: Vec<Task>`) and by reference in
//! the same FRB-scanned module is rejected as ambiguous (opaque vs mirrored)
//! by flutter_rust_bridge's codegen.

use crate::model::{Calendar, Project, Relation, Resource, Task};

impl Project {
    /// Find a task by its permanent `unique_id`, as referenced by
    /// `Relation::predecessor_task_unique_id`, `Assignment::task_unique_id`,
    /// and `Task::parent_task_unique_id`. This is an O(n) scan over
    /// `tasks`; for many repeated lookups, build your own
    /// `HashMap<i32, &Task>` from `tasks` instead.
    ///
    /// Not `task_by_id`: `Task::id` is the task's row position, which
    /// changes when tasks are reordered or inserted. Every cross-reference
    /// in this model uses `unique_id`, not `id`.
    pub fn task_by_unique_id(&self, unique_id: i32) -> Option<&Task> {
        self.tasks.iter().find(|t| t.unique_id == unique_id)
    }

    /// Find a resource by its permanent `unique_id`, as referenced by
    /// `Assignment::resource_unique_id`. O(n); see
    /// [`Project::task_by_unique_id`] for the same caveat on repeated
    /// lookups.
    pub fn resource_by_unique_id(&self, unique_id: i32) -> Option<&Resource> {
        self.resources.iter().find(|r| r.unique_id == unique_id)
    }

    /// Find a calendar by its permanent `unique_id`, as referenced by
    /// `Task::calendar_unique_id`, `Resource::calendar_unique_id`, and
    /// `Calendar::base_calendar_unique_id`. O(n); see
    /// [`Project::task_by_unique_id`] for the same caveat on repeated
    /// lookups.
    pub fn calendar_by_unique_id(&self, unique_id: i32) -> Option<&Calendar> {
        self.calendars.iter().find(|c| c.unique_id == unique_id)
    }

    /// Relations where the task named by `unique_id` is the predecessor,
    /// i.e. the dependencies this task drives. MPXJ derives this as
    /// `Task.getSuccessors()`; zaf-mpp only stores the reverse direction
    /// (`Task::predecessors`, attached to the successor task) since every
    /// `Relation` already carries both ends, so this is reconstructed by
    /// scanning every task's predecessor list. O(total relations in the
    /// project).
    pub fn successors_of(&self, unique_id: i32) -> Vec<&Relation> {
        self.tasks
            .iter()
            .flat_map(|t| t.predecessors.iter())
            .filter(|r| r.predecessor_task_unique_id == unique_id)
            .collect()
    }
}
