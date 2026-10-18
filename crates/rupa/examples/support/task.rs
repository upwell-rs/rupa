use rupa::prelude::*;

pub const SCHEMA: &str =
    "CREATE TABLE tasks (id INTEGER PRIMARY KEY, title TEXT NOT NULL, done BOOLEAN NOT NULL)";

#[derive(Debug, Clone, PartialEq, Entity, Gettable, Insertable, Updatable, Deletable)]
#[entity(table = "tasks")]
pub struct Task {
    #[id(generated)]
    pub id: i64,
    pub title: String,
    pub done: bool,
}

#[derive(Insertable)]
#[insertable(entity = Task)]
pub struct NewTask {
    pub title: String,
    pub done: bool,
}

#[derive(Updatable)]
#[updatable(entity = Task)]
pub struct TaskPatch {
    #[id]
    pub id: i64,
    pub done: Option<bool>,
}
