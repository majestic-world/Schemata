mod common;

use common::TestDb;
use schematadb_core::{
    CellValue, ColumnDesign, Engine, ErrorKind, ForeignKeyDesign, IndexDesign, IndexKind, IndexMethod, ObjectKind, PageRequest, SortSpec,
    TableDesign, TableOptions, connect,
};

fn index(name: &str, column: &str, kind: IndexKind) -> IndexDesign {
    IndexDesign { name: name.into(), columns: vec![column.into()], kind, method: None, comment: None }
}

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
        indexes: vec![index("uq_sku", "sku", IndexKind::Unique)],
        foreign_keys: vec![ForeignKeyDesign {
            name: "fk_product_category".into(),
            columns: vec!["category_id".into()],
            ref_table: "category".into(),
            ref_schema: None,
            ref_columns: vec!["id".into()],
            on_update: "NO ACTION".into(),
            on_delete: "CASCADE".into(),
        }],
        options: TableOptions { comment: (t.engine != Engine::Sqlite).then(|| "Products".to_owned()), ..TableOptions::default() },
        primary_key_order: vec![],
    };
    apply(&t, None, &design).await;

    let info = t.driver.table_info(&t.db, "product").await.unwrap();
    assert_eq!(
        info.columns.iter().map(|c| (c.name.as_str(), c.nullable, c.primary_key, c.auto_increment)).collect::<Vec<_>>(),
        vec![("id", false, true, true), ("sku", false, false, false), ("status", true, false, false), ("category_id", true, false, false),]
    );
    assert_eq!(info.columns[2].default.as_deref(), Some("'new'"));
    assert!(info.indexes.iter().any(|i| i.kind == IndexKind::Unique && i.columns == vec!["sku".to_string()]));
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
    design.indexes.push(index("ix_age", "age", IndexKind::Normal));

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

async fn index_kinds_methods_and_comments(t: TestDb) {
    if t.engine == Engine::Sqlite {
        t.exec("CREATE TABLE doc (id INTEGER PRIMARY KEY, body TEXT)").await;
        let original = t.driver.table_design(&t.db, "doc").await.unwrap();
        let mut design = original.clone();
        design.indexes.push(index("", "body", IndexKind::Normal));
        apply(&t, Some(&original), &design).await;
        let info = t.driver.table_info(&t.db, "doc").await.unwrap();
        assert_eq!(info.indexes.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), vec!["idx_body"], "unnamed index gets idx_<column>");

        let original = t.driver.table_design(&t.db, "doc").await.unwrap();
        let mut design = original.clone();
        design.indexes.push(index("ft_body", "body", IndexKind::Fulltext));
        let err = t.driver.plan_table(&t.db, Some(&original), &design).await.unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation);
        return;
    }
    t.exec("CREATE TABLE doc (id INT PRIMARY KEY, code VARCHAR(10) NOT NULL, title VARCHAR(80), body TEXT) ENGINE=InnoDB").await;
    let original = t.driver.table_design(&t.db, "doc").await.unwrap();
    let mut design = original.clone();
    design.indexes = vec![
        IndexDesign {
            method: Some(IndexMethod::Btree),
            comment: Some("Lookup by code".into()),
            ..index("uq_code", "code", IndexKind::Unique)
        },
        // Unnamed, no method: becomes `idx_title` using BTREE. A second unnamed index on it gets `idx_title_2`.
        index("", "title", IndexKind::Normal),
        index("", "title", IndexKind::Unique),
        index("ft_body", "body", IndexKind::Fulltext),
    ];
    apply(&t, Some(&original), &design).await;

    let info = t.driver.table_info(&t.db, "doc").await.unwrap();
    let read = |name: &str| info.indexes.iter().find(|i| i.name == name).unwrap_or_else(|| panic!("{name} missing")).clone();
    let uq = read("uq_code");
    assert_eq!((uq.kind, uq.method, uq.comment.as_deref()), (IndexKind::Unique, Some(IndexMethod::Btree), Some("Lookup by code")));
    let title = read("idx_title");
    assert_eq!((title.kind, title.method), (IndexKind::Normal, Some(IndexMethod::Btree)));
    assert_eq!(read("idx_title_2").kind, IndexKind::Unique);
    let ft = read("ft_body");
    assert_eq!((ft.kind, ft.method), (IndexKind::Fulltext, None));

    // A changed comment recreates only that index; the rest of the read-back design is unchanged.
    let original = t.driver.table_design(&t.db, "doc").await.unwrap();
    assert!(t.driver.plan_table(&t.db, Some(&original), &original).await.unwrap().is_empty());
    let mut design = original.clone();
    design.indexes.iter_mut().find(|i| i.name == "uq_code").unwrap().comment = Some("Business key".into());
    let ddl = t.driver.plan_table(&t.db, Some(&original), &design).await.unwrap();
    assert!(ddl[0].contains("DROP INDEX `uq_code`") && !ddl[0].contains("ft_body"), "{}", ddl[0]);
    apply(&t, Some(&original), &design).await;
    let info = t.driver.table_info(&t.db, "doc").await.unwrap();
    assert_eq!(info.indexes.iter().find(|i| i.name == "uq_code").unwrap().comment.as_deref(), Some("Business key"));
}
scenario!(index_kinds_methods_and_comments);

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

/// Empty Table deletes the rows and keeps the autoincrement counter; Truncate Table also resets it.
async fn drops_empties_truncates_and_renames(t: TestDb) {
    let auto = if t.engine == Engine::Sqlite { "INTEGER PRIMARY KEY AUTOINCREMENT" } else { "INT AUTO_INCREMENT PRIMARY KEY" };
    t.exec(&format!("CREATE TABLE a (id {auto}, v INT); INSERT INTO a (v) VALUES (1), (2); CREATE TABLE b (id INT);")).await;
    let next_id = async || {
        t.exec("INSERT INTO a (v) VALUES (0)").await;
        let req = PageRequest { offset: 0, limit: 10, sort: Some(SortSpec { column: "id".into(), desc: true }), filters: vec![] };
        t.driver.fetch_page(&t.db, "a", &req).await.unwrap().rows[0][0].clone()
    };

    t.driver.empty_table(&t.db, "a").await.unwrap();
    assert_eq!(t.driver.count_rows(&t.db, "a", &[]).await.unwrap(), 0);
    assert_eq!(next_id().await, CellValue::Int(3), "emptying keeps the counter");
    t.driver.truncate_table(&t.db, "a").await.unwrap();
    assert_eq!(t.driver.count_rows(&t.db, "a", &[]).await.unwrap(), 0);
    assert_eq!(next_id().await, CellValue::Int(1), "truncating resets the counter");

    t.driver.rename_table(&t.db, "a", "a2").await.unwrap();
    t.driver.drop_object(&t.db, "b", ObjectKind::Table).await.unwrap();
    let objects: Vec<String> = t.driver.list_objects(&t.db).await.unwrap().into_iter().map(|o| o.name).collect();
    assert_eq!(objects, vec!["a2"]);
}
scenario!(drops_empties_truncates_and_renames);

/// MySQL/MariaDB DDL names tables without the database; applying it must still target the chosen database,
/// even on a connection whose default database is a different one (or none).
async fn unqualified_ddl_lands_in_the_target_database(t: TestDb) {
    let Some(server) = &t.server else { return };
    t.exec("CREATE TABLE category (id INT PRIMARY KEY)").await;
    let mut id = col("id", "INT");
    id.primary_key = true;
    id.nullable = false;
    let design = TableDesign {
        name: "product".into(),
        columns: vec![id, col("category_id", "INT")],
        indexes: vec![],
        foreign_keys: vec![ForeignKeyDesign {
            name: "fk_product_category".into(),
            columns: vec!["category_id".into()],
            ref_table: "category".into(),
            ref_schema: None,
            ref_columns: vec!["id".into()],
            on_update: "NO ACTION".into(),
            on_delete: "NO ACTION".into(),
        }],
        options: TableOptions::default(),
        primary_key_order: vec![],
    };
    let other = connect(&server.params(None)).await.unwrap();
    let create = other.plan_table(&t.db, None, &design).await.unwrap();
    assert!(create[0].starts_with("CREATE TABLE `product`"), "{}", create[0]);
    other.apply_ddl(&t.db, &create).await.unwrap();

    let original = t.driver.table_design(&t.db, "product").await.unwrap();
    let mut renamed = original.clone();
    renamed.name = "item".into();
    other.apply_ddl(&t.db, &other.plan_table(&t.db, Some(&original), &renamed).await.unwrap()).await.unwrap();
    let info = t.driver.table_info(&t.db, "item").await.unwrap();
    assert_eq!(info.foreign_keys[0].ref_table, "category");
}
scenario!(unqualified_ddl_lands_in_the_target_database);

async fn sqlite_unrepresentable_rebuilds_are_rejected_without_changes(t: TestDb) {
    if t.engine != Engine::Sqlite {
        return;
    }
    for (name, definition, suffix) in [
        ("checked", "id INTEGER PRIMARY KEY, value INT CHECK (value > 0)", ""),
        ("collated", "id INTEGER PRIMARY KEY, value TEXT COLLATE NOCASE", ""),
        ("strict_table", "id INTEGER PRIMARY KEY, value INT", " STRICT"),
    ] {
        t.exec(&format!("CREATE TABLE {name} ({definition}){suffix};")).await;
        t.exec(&format!("INSERT INTO {name} VALUES (1, 5)")).await;
        let original = t.driver.table_design(&t.db, name).await.unwrap();
        let before = t.driver.table_info(&t.db, name).await.unwrap().create_sql;
        let mut changed = original.clone();
        changed.columns[1].nullable = !changed.columns[1].nullable;
        assert!(t.driver.plan_table(&t.db, Some(&original), &changed).await.is_err());
        assert_eq!(t.driver.table_info(&t.db, name).await.unwrap().create_sql, before);
        assert_eq!(all_rows(&t, name, "id").await.len(), 1);
    }
}
scenario!(sqlite_unrepresentable_rebuilds_are_rejected_without_changes);

async fn sqlite_rebuild_replacing_every_column_is_rejected(t: TestDb) {
    if t.engine != Engine::Sqlite {
        return;
    }
    t.exec("CREATE TABLE swap (a INTEGER, b TEXT); INSERT INTO swap VALUES (1, 'x'), (2, 'y');").await;
    let original = t.driver.table_design(&t.db, "swap").await.unwrap();
    let mut replaced = original.clone();
    replaced.columns = vec![col("c", "INTEGER"), col("d", "TEXT")];
    let error = t.driver.plan_table(&t.db, Some(&original), &replaced).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Validation);
    assert_eq!(names(&t, "swap").await, vec!["a", "b"]);
    assert_eq!(all_rows(&t, "swap", "a").await.len(), 2);
}
scenario!(sqlite_rebuild_replacing_every_column_is_rejected);

async fn sqlite_rebuild_with_rename_and_dependents_is_rejected(t: TestDb) {
    if t.engine != Engine::Sqlite {
        return;
    }
    t.exec(
        "CREATE TABLE item (id INTEGER PRIMARY KEY, label TEXT, weight TEXT);
         CREATE TABLE audit (msg TEXT);
         CREATE TRIGGER item_ins AFTER INSERT ON item BEGIN INSERT INTO audit VALUES (NEW.label); END;
         INSERT INTO item VALUES (1, 'a', '1');",
    )
    .await;
    let original = t.driver.table_design(&t.db, "item").await.unwrap();
    let mut both = original.clone();
    both.columns.iter_mut().find(|c| c.name == "label").unwrap().name = "title".into();
    both.columns.iter_mut().find(|c| c.name == "weight").unwrap().data_type = "REAL".into();
    let error = t.driver.plan_table(&t.db, Some(&original), &both).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Validation);
    assert_eq!(names(&t, "item").await, vec!["id", "label", "weight"]);

    // A rename on its own uses ALTER TABLE, which rewrites the trigger.
    let mut rename = original.clone();
    rename.columns.iter_mut().find(|c| c.name == "label").unwrap().name = "title".into();
    apply(&t, Some(&original), &rename).await;
    t.exec("INSERT INTO item (title, weight) VALUES ('b', '2')").await;
    // 'a' came from the original insert; 'b' proves the trigger now reads the renamed column.
    assert_eq!(all_rows(&t, "audit", "msg").await, vec![vec![CellValue::Text("a".into())], vec![CellValue::Text("b".into())]]);
}
scenario!(sqlite_rebuild_with_rename_and_dependents_is_rejected);

async fn sqlite_rebuild_preserves_autoincrement_high_water_mark(t: TestDb) {
    if t.engine != Engine::Sqlite {
        return;
    }
    t.exec(
        "CREATE TABLE sequence_test (id INTEGER PRIMARY KEY AUTOINCREMENT, value TEXT, obsolete INT); \
            INSERT INTO sequence_test (id, value) VALUES (100, 'old'); DELETE FROM sequence_test",
    )
    .await;
    let original = t.driver.table_design(&t.db, "sequence_test").await.unwrap();
    let mut changed = original.clone();
    changed.columns.pop();
    let plan = t.driver.plan_table(&t.db, Some(&original), &changed).await.unwrap();
    // The reviewed plan must preserve sequence changes made before it is actually applied too.
    t.exec("INSERT INTO sequence_test (id, value) VALUES (200, 'new'); DELETE FROM sequence_test").await;
    t.driver.apply_ddl(&t.db, &plan).await.unwrap();
    t.exec("INSERT INTO sequence_test (value) VALUES ('next')").await;
    assert_eq!(all_rows(&t, "sequence_test", "id").await[0][0], CellValue::Int(201));
}
scenario!(sqlite_rebuild_preserves_autoincrement_high_water_mark);

async fn mysql_column_attributes_are_not_silently_removed(t: TestDb) {
    if t.engine == Engine::Sqlite {
        return;
    }
    t.exec(
        "CREATE TABLE attributes (id INT PRIMARY KEY, updated TIMESTAMP DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP); \
            INSERT INTO attributes (id) VALUES (1)",
    )
    .await;
    let original = t.driver.table_design(&t.db, "attributes").await.unwrap();
    let before = t.driver.table_info(&t.db, "attributes").await.unwrap().create_sql;
    let mut changed = original.clone();
    changed.columns[1].name = "renamed".into();
    assert!(t.driver.plan_table(&t.db, Some(&original), &changed).await.is_err());
    assert_eq!(t.driver.table_info(&t.db, "attributes").await.unwrap().create_sql, before);
}
scenario!(mysql_column_attributes_are_not_silently_removed);

async fn mysql_backslash_defaults_survive_column_changes(t: TestDb) {
    if t.engine == Engine::Sqlite {
        return;
    }
    t.exec(r"CREATE TABLE escaped_default (id INT PRIMARY KEY, value VARCHAR(100) DEFAULT 'C:\\temp\\new')").await;
    let original = t.driver.table_design(&t.db, "escaped_default").await.unwrap();
    let mut changed = original.clone();
    changed.columns[1].data_type = "VARCHAR(120)".into();
    apply(&t, Some(&original), &changed).await;
    t.exec("INSERT INTO escaped_default (id) VALUES (1)").await;
    assert_eq!(all_rows(&t, "escaped_default", "id").await[0][1], CellValue::Text(r"C:\temp\new".into()));
}
scenario!(mysql_backslash_defaults_survive_column_changes);

async fn designer_refuses_an_outdated_original(t: TestDb) {
    t.exec(&format!("CREATE TABLE t (id {} PRIMARY KEY, a VARCHAR(20))", int_type(&t))).await;
    let original = t.driver.table_design(&t.db, "t").await.unwrap();
    t.exec("ALTER TABLE t ADD COLUMN notes VARCHAR(50)").await;
    t.exec("INSERT INTO t (id, a, notes) VALUES (1, 'x', 'keep me')").await;
    let mut design = original.clone();
    design.columns[1].data_type = "VARCHAR(30)".into();
    let err = t.driver.plan_table(&t.db, Some(&original), &design).await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation, "{err:?}");
    assert_eq!(names(&t, "t").await, vec!["id", "a", "notes"]);
}
scenario!(designer_refuses_an_outdated_original);

async fn sqlite_rebuild_keeps_rowids(t: TestDb) {
    if t.engine != Engine::Sqlite {
        return;
    }
    t.exec("CREATE TABLE t (name TEXT, qty INT); INSERT INTO t VALUES ('a', 1), ('b', 2), ('c', 3), ('d', 4); DELETE FROM t WHERE name IN ('b', 'c')").await;
    let original = t.driver.table_design(&t.db, "t").await.unwrap();
    let mut design = original.clone();
    design.columns[1].data_type = "BIGINT".into();
    apply(&t, Some(&original), &design).await;
    let rows = t.driver.execute_script(Some("main"), "SELECT rowid, name FROM t ORDER BY rowid", 10, None).await.unwrap();
    let schematadb_core::StatementOutcome::Rows { rows, .. } = &rows.outcomes[0] else { panic!("no rows") };
    assert_eq!(rows, &vec![vec![CellValue::Int(1), CellValue::Text("a".into())], vec![CellValue::Int(4), CellValue::Text("d".into())]]);
}
scenario!(sqlite_rebuild_keeps_rowids);

async fn composite_primary_key_order_is_kept(t: TestDb) {
    let int = int_type(&t);
    t.exec(&format!("CREATE TABLE m (a {int} NOT NULL, b {int} NOT NULL, note VARCHAR(10), PRIMARY KEY (b, a))")).await;
    let info = t.driver.table_info(&t.db, "m").await.unwrap();
    assert_eq!(info.primary_key, vec!["b", "a"]);
    let original = t.driver.table_design(&t.db, "m").await.unwrap();
    let mut design = original.clone();
    design.columns[2].data_type = "VARCHAR(40)".into();
    apply(&t, Some(&original), &design).await;
    assert_eq!(t.driver.table_info(&t.db, "m").await.unwrap().primary_key, vec!["b", "a"]);
}
scenario!(composite_primary_key_order_is_kept);

async fn composite_primary_key_order_is_kept_when_a_column_joins_the_key(t: TestDb) {
    let int = int_type(&t);
    t.exec(&format!("CREATE TABLE mk (a {int} NOT NULL, b {int} NOT NULL, c {int} NOT NULL, PRIMARY KEY (b, a))")).await;
    let original = t.driver.table_design(&t.db, "mk").await.unwrap();
    let mut design = original.clone();
    design.columns[2].primary_key = true;
    design.primary_key_order.push("c".into());
    apply(&t, Some(&original), &design).await;
    assert_eq!(t.driver.table_info(&t.db, "mk").await.unwrap().primary_key, vec!["b", "a", "c"]);
}
scenario!(composite_primary_key_order_is_kept_when_a_column_joins_the_key);

async fn sqlite_rebuild_with_rename_keeps_foreign_keys_pointing_at_the_table(t: TestDb) {
    if t.engine != Engine::Sqlite {
        return;
    }
    t.exec(
        "CREATE TABLE parent (id INTEGER PRIMARY KEY, up INTEGER REFERENCES parent(id), w TEXT);
         INSERT INTO parent VALUES (1, NULL, '1');
         CREATE TABLE child (id INTEGER PRIMARY KEY, parent_id INTEGER REFERENCES parent(id));",
    )
    .await;
    let original = t.driver.table_design(&t.db, "parent").await.unwrap();
    let mut design = original.clone();
    design.name = "folks".into();
    design.columns[2].data_type = "REAL".into();
    let ddl = t.driver.plan_table(&t.db, Some(&original), &design).await.unwrap();
    assert!(ddl.iter().any(|s| s.contains("INSERT INTO")), "type changes need a rebuild: {ddl:?}");
    t.driver.apply_ddl(&t.db, &ddl).await.unwrap();

    assert_eq!(t.driver.table_info(&t.db, "child").await.unwrap().foreign_keys[0].ref_table, "folks");
    assert_eq!(t.driver.table_info(&t.db, "folks").await.unwrap().foreign_keys[0].ref_table, "folks");
    t.exec("PRAGMA foreign_keys = ON; INSERT INTO child VALUES (1, 1); INSERT INTO folks VALUES (2, 1, 2.5);").await;
    assert_eq!(all_rows(&t, "child", "id").await, vec![vec![CellValue::Int(1), CellValue::Int(1)]]);
}
scenario!(sqlite_rebuild_with_rename_keeps_foreign_keys_pointing_at_the_table);

async fn enum_value_case_changes_produce_ddl(t: TestDb) {
    if t.engine == Engine::Sqlite {
        return;
    }
    t.exec("CREATE TABLE flag (id INT PRIMARY KEY, state ENUM('on','off'))").await;
    let original = t.driver.table_design(&t.db, "flag").await.unwrap();
    let mut design = original.clone();
    design.columns[1].data_type = "ENUM('ON','off')".into();
    apply(&t, Some(&original), &design).await;
    let info = t.driver.table_info(&t.db, "flag").await.unwrap();
    assert!(info.columns[1].data_type.contains("'ON'"), "{}", info.columns[1].data_type);
    let read_back = t.driver.table_design(&t.db, "flag").await.unwrap();
    let mut upper = read_back.clone();
    upper.columns[1].data_type = upper.columns[1].data_type.to_uppercase().replace("'OFF'", "'off'");
    assert!(t.driver.plan_table(&t.db, Some(&read_back), &upper).await.unwrap().is_empty(), "keyword case is not a change");
}
scenario!(enum_value_case_changes_produce_ddl);
