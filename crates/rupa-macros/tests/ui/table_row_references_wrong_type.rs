rupa::table!(Authors = "authors" { #[id] id: i64 });

rupa::table!(Posts = "posts" as PostRow {
    #[id]
    id: i64,
    #[column(references = Authors)]
    author_id: String,
});

fn main() {}
