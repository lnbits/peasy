//! Window-owned work, independent of GTK so close/reopen races can be tested.
use peasy_client::Cancellation;

#[derive(Clone, Default)]
pub struct Task {
    pub work: Cancellation,
    pub view: Cancellation,
}

#[derive(Default)]
pub struct Tasks(Vec<Task>);

impl Tasks {
    pub fn start(&mut self) -> Task {
        self.0.retain(|task| !task.view.is_cancelled());
        let task = Task::default();
        self.0.push(task.clone());
        task
    }

    pub fn close(&mut self) {
        for task in self.0.drain(..) {
            task.view.cancel();
            task.work.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn close_cancels_work_and_new_window_has_fresh_lifetime() {
        let mut tasks = Tasks::default();
        let old = tasks.start();
        tasks.close();
        let new = tasks.start();
        assert!(old.work.is_cancelled());
        assert!(old.view.is_cancelled());
        assert!(!new.work.is_cancelled());
    }
    #[test]
    fn started_mutation_finishes_without_touching_reopened_view() {
        let mut tasks = Tasks::default();
        let task = tasks.start();
        task.work.protect().unwrap();
        tasks.close();
        assert!(!task.work.is_cancelled());
        assert!(task.view.is_cancelled());
    }
}
