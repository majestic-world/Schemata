mod common;

use common::TestDb;
use schematadb_core::{
    CellValue, ColumnDesign, Engine, ErrorKind, ForeignKeyDesign, IndexDesign, ObjectKind, PageRequest, SortSpec, TableDesign, TableOptions,
};

fn col(name: &str, data_type: &str) -> ColumnDesign {
    ColumnDesign {
        original_name: None,
        name: name.into(),
        data_type: data_type.into(),
        nullable: true,
        default: None,
        auto_increment: false,
        primary_key: false,
        comment: None,
    }
}

fn int_type(t: &TestDb) -> &'static str {
    if t.engine == Engine::Sqlite { "INTEGER" } else { "INT" }
}

async fn names(t: &TestDb, table: &str) -> Vec<String> {
    t.driver.table_info(&t.db, table).await.unwrap().columns.into_iter().map(|c| c.name).collect()
}

async fn all_rows(t: &TestDb, table: &str, order: &str) -> Vec<Vec<CellValue>> {
    let req = PageRequest { offset: 0, limit: 100, sort: Some(SortSpec { column: order.into(), desc: false }), filters: vec![] };
    let page = t.driver.fetch_page(&t.db, table, &req).await.unwrap();
    page.rows.into_iter().map(|r| r.into_iter().zip(&page.columns).filter(|(_, c)| !c.hidden).map(|(v, _)| v).collect()).collect()
}

async fn apply(t: &TestDb, original: Option<&TableDesign>, design: &TableDesign) {
    let ddl = t.driver.plan_table(&t.db, original, design).await.unwrap();
    assert!(!ddl.is_empty());
    t.driver.apply_ddl(&t.db, &ddl).await.unwrap_or_else(|e| panic!("{e}\n{}", ddl.join(";\n")));
}

async fn creates_table_from_design(t: TestDb) {
    t.exec(&format!("CREATE TABLE category (id {} PRIMARY KEY)", int_type(&t))).await;
    let mut id = col("id", int_type(&t));
    id.primary_key = true;
    id.auto_increment = true;
    id.nullable = false;
    let mut sku = col("sku", "VARCHAR(20)");
    sku.nullable = false;
    let mut status = col("status", "VARCHAR(10)");
    status.default = Some("'new'".into());
    let design = TableDesign {
        name: "product".into(),
        columns: vec![id, sku, status, col("category_id", int_type(&t))],
        indexes: vec![IndexDesign { name: "uq_sku".into(), columns: vec!["sku".into()], unique: true }],
        foreign_keys: vec![ForeignKeyDesign {
            name: "fk_product_category".into(),
            columns: vec!["category_id".into()],
            ref_table: "category".into(),
            ref_columns: vec!["id".into()],
            on_update: "NO ACTION".into(),
            on_delete: "CASCADE".into(),
        }],
        options: TableOptions { comment: (t.engine != Engine::Sqlite).then(|| "Products".to_owned()), ..TableOptions::default() },
    };
    apply(&t, None, &design).await;

    let info = t.driver.table_info(&t.db, "product").await.unwrap();
    assert_eq!(
        info.columns.iter().map(|c| (c.name.as_str(), c.nullable, c.primary_key, c.auto_increment)).collect::<Vec<_>>(),
        vec![("id", false, true, true), ("sku", false, false, false), ("status", true, false, false), ("category_id", true, false, false),]
    );
    assert_eq!(info.columns[2].default.as_deref(), Some("'new'"));
    assert!(info.indexes.iter().any(|i| i.unique && i.columns == vec!["sku".to_string()]));
    assert_eq!(info.foreign_keys[0].on_delete, "CASCADE");
    if t.engine != Engine::Sqlite {
        assert_eq!(info.comment.as_deref(), Some("Products"));
    }
    // Round trip: the design read back is the design written (modulo original names).
    let read_back = t.driver.table_design(&t.db, "product").await.unwrap();
    assert_eq!(read_back.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["id", "sku", "status", "category_id"]);
    assert!(t.driver.plan_table(&t.db, Some(&read_back), &read_back).await.unwrap().is_empty(), "an unchanged design needs no DDL");
}
scenario!(creates_table_from_design);

async fn alters_columns_indexes_and_name(t: TestDb) {
    t.exec(&format!(
        "CREATE TABLE person (id {} PRIMARY KEY, first VARCHAR(20), age INT, legacy VARCHAR(5));
         INSERT INTO person VALUES (1, 'Ana', 30, 'x'), (2, 'Bia', 41, 'y');",
        int_type(&t)
    ))
    .await;
    let original = t.driver.table_design(&t.db, "person").await.unwrap();
    let mut design = original.clone();
    design.name = "people".into();
    design.columns.retain(|c| c.name != "legacy");
    let first = design.columns.iter_mut().find(|c| c.name == "first").unwrap();
    first.name = "first_name".into();
    first.data_type = "VARCHAR(60)".into();
    let mut email = col("email", "VARCHAR(80)");
    email.default = Some("'none'".into());
    // New order: id, age, email, first_name
    let first = design.columns.remove(1);
    design.columns.push(email);
    design.columns.push(first);
    design.indexes.push(IndexDesign { name: "ix_age".into(), columns: vec!["age".into()], unique: false });

    apply(&t, Some(&original), &design).await;

    assert_eq!(names(&t, "people").await, vec!["id", "age", "email", "first_name"]);
    let info = t.driver.table_info(&t.db, "people").await.unwrap();
    assert!(info.columns[3].data_type.to_lowercase().contains("60"), "{}", info.columns[3].data_type);
    assert!(info.indexes.iter().any(|i| i.name == "ix_age"));
    assert_eq!(
        all_rows(&t, "people", "id").await,
        vec![
            vec![CellValue::Int(1), CellValue::Int(30), CellValue::Text("none".into()), CellValue::Text("Ana".into())],
            vec![CellValue::Int(2), CellValue::Int(41), CellValue::Text("none".into()), CellValue::Text("Bia".into())],
        ]
    );
}
scenario!(alters_columns_indexes_and_name);

async fn failed_alteration_leaves_table_untouched(t: TestDb) {
    t.exec(&format!("CREATE TABLE note (id {} PRIMARY KEY, body VARCHAR(20)); INSERT INTO note VALUES (1, NULL), (2, 'x');", int_type(&t)))
        .await;
    let original = t.driver.table_design(&t.db, "note").await.unwrap();
    let mut design = original.clone();
    let body = design.columns.iter_mut().find(|c| c.name == "body").unwrap();
    body.nullable = false; // existing NULL makes this fail
    body.name = "text".into();
    design.columns.push(col("extra", "INT"));
    let ddl = t.driver.plan_table(&t.db, Some(&original), &design).await.unwrap();
    let err = t.driver.apply_ddl(&t.db, &ddl).await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::Sql);
    assert_eq!(names(&t, "note").await, vec!["id", "body"]);
    assert_eq!(all_rows(&t, "note", "id").await.len(), 2);
}
scenario!(failed_alteration_leaves_table_untouched);

async fn sqlite_rebuild_preserves_dependents(t: TestDb) {
    if t.engine != Engine::Sqlite {
        return;
    }
    t.exec(
        "CREATE TABLE owner (id INTEGER PRIMARY KEY);
         INSERT INTO owner VALUES (1);
         CREATE TABLE pet (id INTEGER PRIMARY KEY, owner_id INTEGER REFERENCES owner(id) ON DELETE CASCADE, name TEXT, weight TEXT);
         CREATE INDEX ix_pet_name ON pet(name);
         CREATE TABLE log (msg TEXT);
         CREATE TRIGGER pet_ins AFTER INSERT ON pet BEGIN INSERT INTO log VALUES ('pet ' || NEW.name); END;
         CREATE VIEW heavy AS SELECT name FROM pet WHERE weight > 10;
         INSERT INTO pet VALUES (1, 1, 'Rex', '12.5');",
    )
    .await;
    let original = t.driver.table_design(&t.db, "pet").await.unwrap();
    let mut design = original.clone();
    design.columns.iter_mut().find(|c| c.name == "weight").unwrap().data_type = "REAL".into();
    let ddl = t.driver.plan_table(&t.db, Some(&original), &design).await.unwrap();
    assert!(ddl.iter().any(|s| s.contains("INSERT INTO")), "type changes need a rebuild: {ddl:?}");
    t.driver.apply_ddl(&t.db, &ddl).await.unwrap();

    let info = t.driver.table_info(&t.db, "pet").await.unwrap();
    assert_eq!(info.columns.iter().find(|c| c.name == "weight").unwrap().data_type, "REAL");
    assert!(info.indexes.iter().any(|i| i.name == "ix_pet_name"));
    assert_eq!(info.foreign_keys.len(), 1);
    assert_eq!(
        all_rows(&t, "pet", "id").await,
        vec![vec![CellValue::Int(1), CellValue::Int(1), CellValue::Text("Rex".into()), CellValue::Float(12.5)]]
    );
    // The trigger still fires (and did not fire for the rows copied during the rebuild).
    t.exec("INSERT INTO pet (owner_id, name, weight) VALUES (1, 'Tom', 3)").await;
    assert_eq!(all_rows(&t, "log", "msg").await, vec![vec![CellValue::Text("pet Rex".into())], vec![CellValue::Text("pet Tom".into())]]);
    assert_eq!(t.driver.table_info(&t.db, "heavy").await.unwrap().kind, ObjectKind::View);
    assert_eq!(
        t.driver.fetch_page(&t.db, "heavy", &PageRequest { offset: 0, limit: 10, sort: None, filters: vec![] }).await.unwrap().rows.len(),
        1
    );
}
scenario!(sqlite_rebuild_preserves_dependents);

async fn drops_truncates_and_renames(t: TestDb) {
    t.exec("CREATE TABLE a (id INT PRIMARY KEY); INSERT INTO a VALUES (1), (2); CREATE TABLE b (id INT);").await;
    t.driver.truncate_table(&t.db, "a").await.unwrap();
    assert_eq!(t.driver.count_rows(&t.db, "a", &[]).await.unwrap(), 0);
    t.driver.rename_table(&t.db, "a", "a2").await.unwrap();
    t.driver.drop_object(&t.db, "b", ObjectKind::Table).await.unwrap();
    let objects: Vec<String> = t.driver.list_objects(&t.db).await.unwrap().into_iter().map(|o| o.name).collect();
    assert_eq!(objects, vec!["a2"]);
}
scenario!(drops_truncates_and_renames);
