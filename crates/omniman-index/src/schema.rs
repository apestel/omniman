use tantivy::schema::{Schema, SchemaBuilder, FAST, INDEXED, STORED, STRING, TEXT};

pub struct Fields {
    pub path: tantivy::schema::Field,
    pub filename: tantivy::schema::Field,
    pub parent: tantivy::schema::Field,
    pub mtime: tantivy::schema::Field,
    pub size: tantivy::schema::Field,
    pub mime: tantivy::schema::Field,
}

pub fn build() -> (Schema, Fields) {
    let mut builder = SchemaBuilder::new();

    let path = builder.add_text_field("path", STRING | STORED);
    let filename = builder.add_text_field("filename", TEXT | STORED);
    let parent = builder.add_text_field("parent", TEXT | STORED);
    let mime = builder.add_text_field("mime", STRING | STORED);
    let mtime = builder.add_u64_field("mtime", INDEXED | FAST | STORED);
    let size = builder.add_u64_field("size", INDEXED | FAST | STORED);

    let schema = builder.build();
    let fields = Fields { path, filename, parent, mtime, size, mime };
    (schema, fields)
}
