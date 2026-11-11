mod common;

use common::TestDb;
use projectcat_core::{CellValue, Engine, Filter, FilterOp, Page, PageRequest, SortSpec};

fn text(s: &str) -> CellValue {
    CellValue::Text(s.into())
}

async fn seed_people(t: &TestDb) {
    t.exec(
        "CREATE TABLE people (id INT PRIMARY KEY, name VARCHAR(40), city VARCHAR(40), age INT);
         INSERT INTO people VALUES
           (1, 'Ana', 'Lisbon', 31), (2, 'Bruno', 'Porto', 25), (3, 'Carla', NULL, 40),
           (4, 'Davi', 'Lisbon', 19), (5, 'Eva', 'Braga', 52), (6, '50%_off', 'Porto', 33);",
    )
    .await;
}

fn column(page: &Page, name: &str) -> Vec<CellValue> {
    let idx = page.columns.iter().position(|c| c.name == name).unwrap();
    page.rows.iter().map(|r| r[idx].clone()).collect()
}

fn filter(column: &str, op: FilterOp, value: Option<CellValue>) -> Filter {
    Filter { column: column.into(), op, value }
}

async fn pages_and_sorts_server_side(t: TestDb) {
    seed_people(&t).await;
    // Ages descending: Eva 52, Carla 40, 50%_off 33, Ana 31, Bruno 25, Davi 19.
    let req = PageRequest { offset: 1, limit: 2, sort: Some(SortSpec { column: "age".into(), desc: true }), filters: vec![] };
    let page = t.driver.fetch_page(&t.db, "people", &req).await.unwrap();
    assert_eq!(column(&page, "name"), vec![text("Carla"), text("50%_off")]);
    assert_eq!(page.columns.iter().filter(|c| !c.hidden).map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["id", "name", "city", "age"]);
    assert_eq!(t.driver.count_rows(&t.db, "people", &[]).await.unwrap(), 6);
}
scenario!(pages_and_sorts_server_side);

async fn filters_combine_with_and(t: TestDb) {
    seed_people(&t).await;
    let by_id = Some(SortSpec { column: "id".into(), desc: false });
    let run = |filters: Vec<Filter>| {
        let req = PageRequest { offset: 0, limit: 100, sort: by_id.clone(), filters };
        let driver = t.driver.clone();
        let db = t.db.clone();
        async move { driver.fetch_page(&db, "people", &req).await.unwrap() }
    };

    let lisbon_over_20 =
        run(vec![filter("city", FilterOp::Eq, Some(text("Lisbon"))), filter("age", FilterOp::Gt, Some(CellValue::Int(20)))]).await;
    assert_eq!(column(&lisbon_over_20, "name"), vec![text("Ana")]);

    let no_city = run(vec![filter("city", FilterOp::IsNull, None)]).await;
    assert_eq!(column(&no_city, "name"), vec![text("Carla")]);

    // LIKE wildcards in the user's value are matched literally.
    let literal = run(vec![filter("name", FilterOp::Contains, Some(text("%_")))]).await;
    assert_eq!(column(&literal, "name"), vec![text("50%_off")]);

    let starts = run(vec![filter("name", FilterOp::StartsWith, Some(text("b")))]).await;
    assert_eq!(column(&starts, "name"), vec![text("Bruno")]);

    let not_porto = run(vec![filter("city", FilterOp::NotEq, Some(text("Porto"))), filter("city", FilterOp::IsNotNull, None)]).await;
    assert_eq!(column(&not_porto, "name"), vec![text("Ana"), text("Davi"), text("Eva")]);

    let filters = vec![filter("age", FilterOp::Lte, Some(CellValue::Int(25)))];
    assert_eq!(t.driver.count_rows(&t.db, "people", &filters).await.unwrap(), 2);
}
scenario!(filters_combine_with_and);

async fn values_keep_precision_and_null(t: TestDb) {
    let sqlite = t.engine == Engine::Sqlite;
    if sqlite {
        t.exec(
            "CREATE TABLE v (id INTEGER PRIMARY KEY, big INTEGER, amount TEXT, bin BLOB, dt TEXT, js TEXT, blank TEXT, missing TEXT, f REAL);
             INSERT INTO v VALUES (1, 9223372036854775807, '12345678901234567890.0123456789', X'00FF10', '2024-03-16 10:28:41', '{\"a\":1}', '', NULL, 1.5);",
        )
        .await;
    } else {
        t.exec(
            "CREATE TABLE v (id INT PRIMARY KEY, big BIGINT, ubig BIGINT UNSIGNED, amount DECIMAL(30,10), bin VARBINARY(10), dt DATETIME, d DATE, tm TIME,
                             js JSON, blank VARCHAR(5), missing VARCHAR(5), f DOUBLE, small INT, bits BIT(8));
             INSERT INTO v VALUES (1, 9223372036854775807, 18446744073709551615, 12345678901234567890.0123456789, X'00FF10',
                                   '2024-03-16 10:28:41', '2024-03-16', '-838:59:59', '{\"a\": 1}', '', NULL, 1.5, 42, b'101');",
        )
        .await;
    }
    let req = PageRequest { offset: 0, limit: 10, sort: None, filters: vec![] };
    let page = t.driver.fetch_page(&t.db, "v", &req).await.unwrap();
    let get = |name: &str| column(&page, name)[0].clone();
    assert_eq!(get("big"), CellValue::BigInt("9223372036854775807".into()));
    assert_eq!(get("bin"), CellValue::Bytes { preview_hex: "00FF10".into(), len: 3 });
    assert_eq!(get("blank"), text(""));
    assert_eq!(get("missing"), CellValue::Null);
    assert_eq!(get("f"), CellValue::Float(1.5));
    assert_eq!(get("dt"), if sqlite { text("2024-03-16 10:28:41") } else { CellValue::DateTime("2024-03-16 10:28:41".into()) });
    if sqlite {
        assert_eq!(get("amount"), text("12345678901234567890.0123456789"));
        return;
    }
    assert_eq!(get("ubig"), CellValue::BigInt("18446744073709551615".into()));
    assert_eq!(get("amount"), CellValue::Decimal("12345678901234567890.0123456789".into()));
    assert_eq!(get("d"), CellValue::DateTime("2024-03-16".into()));
    assert_eq!(get("tm"), CellValue::DateTime("-838:59:59".into()));
    assert_eq!(get("small"), CellValue::Int(42));
    assert_eq!(get("bits"), CellValue::Int(5));
    match get("js") {
        CellValue::Json(j) | CellValue::Text(j) => {
            assert_eq!(serde_json::from_str::<serde_json::Value>(&j).unwrap(), serde_json::json!({"a": 1}))
        }
        other => panic!("{other:?}"),
    }
}
scenario!(values_keep_precision_and_null);

async fn sqlite_tables_without_key_expose_hidden_rowid(t: TestDb) {
    if t.engine != Engine::Sqlite {
        return;
    }
    t.exec("CREATE TABLE loose (a TEXT); INSERT INTO loose VALUES ('x');").await;
    let page = t.driver.fetch_page(&t.db, "loose", &PageRequest { offset: 0, limit: 10, sort: None, filters: vec![] }).await.unwrap();
    assert!(page.columns[0].hidden);
    assert_eq!(page.rows[0], vec![CellValue::Int(1), text("x")]);
}
scenario!(sqlite_tables_without_key_expose_hidden_rowid);
