use rupa_spike_column_kind::Entity;

#[derive(Entity)]
#[entity(table = "t")]
struct Doc<T: serde::Serialize + serde::de::DeserializeOwned> {
    #[id]
    id: i64,
    body: T,
}

fn main() {}
