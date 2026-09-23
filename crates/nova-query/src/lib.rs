//! Query engine and NQL parser for NOVA DB.

pub mod ast;
pub mod evaluator;
pub mod lexer;
pub mod parser;
pub mod planner;

pub use ast::{Assignment, BinaryOp, Expr, SortClause, SortDirection, Statement, UnaryOp};
pub use evaluator::Evaluator;
pub use lexer::{Lexer, Span, Token, TokenKind};
pub use parser::Parser;
pub use planner::{QueryPlan, QueryPlanner};

use nova_core::error::Result;

/// Parse an NQL query string into an executable AST Statement.
pub fn parse_nql(query: &str) -> Result<Statement> {
    let mut lexer = Lexer::new(query);
    let tokens = lexer.tokenize()?;
    let mut parser = Parser::new(tokens);
    parser.parse_statement()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nova_core::document::Document;
    use nova_core::value::Value;
    use nova_index::{HashIndex, IndexManager, OrderedIndex};

    #[test]
    fn test_nql_find_queries() {
        let stmt =
            parse_nql("FIND users WHERE age > 18 SORT created_at DESC LIMIT 20 OFFSET 10").unwrap();
        match stmt {
            Statement::Find {
                collection,
                filter,
                sort,
                limit,
                offset,
            } => {
                assert_eq!(collection, "users");
                assert!(filter.is_some());
                let sort = sort.unwrap();
                assert_eq!(sort.field, "created_at");
                assert_eq!(sort.direction, SortDirection::Desc);
                assert_eq!(limit, Some(20));
                assert_eq!(offset, Some(10));
            }
            _ => panic!("Expected Find statement"),
        }
    }

    #[test]
    fn test_nql_insert_query() {
        let q = r#"INSERT INTO users VALUES ({ "name": "Sunrit", "age": 28, "active": true })"#;
        let stmt = parse_nql(q).unwrap();
        match stmt {
            Statement::Insert {
                collection,
                document,
            } => {
                assert_eq!(collection, "users");
                assert_eq!(
                    document.get("name"),
                    Some(&Value::String("Sunrit".to_string()))
                );
                assert_eq!(document.get("age"), Some(&Value::Int(28)));
                assert_eq!(document.get("active"), Some(&Value::Bool(true)));
            }
            _ => panic!("Expected Insert statement"),
        }
    }

    #[test]
    fn test_nql_update_query() {
        let stmt = parse_nql(r#"UPDATE users SET role = "lead", age = age + 1 WHERE id == "u100""#)
            .unwrap();
        match stmt {
            Statement::Update {
                collection,
                assignments,
                filter,
            } => {
                assert_eq!(collection, "users");
                assert_eq!(assignments.len(), 2);
                assert_eq!(assignments[0].field, "role");
                assert!(filter.is_some());
            }
            _ => panic!("Expected Update statement"),
        }
    }

    #[test]
    fn test_nql_remove_watch_count() {
        let stmt1 = parse_nql("REMOVE users WHERE inactive == true").unwrap();
        assert!(matches!(stmt1, Statement::Remove { .. }));

        let stmt2 = parse_nql("WATCH users WHERE country == \"IN\"").unwrap();
        assert!(matches!(stmt2, Statement::Watch { .. }));

        let stmt3 = parse_nql("COUNT users WHERE age >= 21").unwrap();
        assert!(matches!(stmt3, Statement::Count { .. }));
    }

    #[test]
    fn test_nql_index_ddl() {
        let stmt1 = parse_nql("CREATE INDEX users.email TYPE hash").unwrap();
        match stmt1 {
            Statement::CreateIndex {
                collection,
                field,
                index_type,
            } => {
                assert_eq!(collection, "users");
                assert_eq!(field, "email");
                assert_eq!(index_type, nova_index::IndexType::Hash);
            }
            _ => panic!("Expected CreateIndex"),
        }

        let stmt2 = parse_nql("DROP INDEX users.email").unwrap();
        match stmt2 {
            Statement::DropIndex { collection, field } => {
                assert_eq!(collection, "users");
                assert_eq!(field, "email");
            }
            _ => panic!("Expected DropIndex"),
        }
    }

    #[test]
    fn test_evaluator_expressions() {
        let mut doc = Document::with_id("u1");
        doc.insert("age", 25);
        doc.insert("score", 92.5);
        doc.insert("country", "IN");

        let q1 = parse_nql("FIND users WHERE age > 20 && country == \"IN\"").unwrap();
        if let Statement::Find { filter, .. } = q1 {
            assert!(Evaluator::matches_filter(&filter, &doc).unwrap());
        }

        let q2 = parse_nql("FIND users WHERE age BETWEEN 30 AND 40").unwrap();
        if let Statement::Find { filter, .. } = q2 {
            assert!(!Evaluator::matches_filter(&filter, &doc).unwrap());
        }

        let q3 = parse_nql("FIND users WHERE country IN (\"US\", \"IN\", \"UK\")").unwrap();
        if let Statement::Find { filter, .. } = q3 {
            assert!(Evaluator::matches_filter(&filter, &doc).unwrap());
        }
    }

    #[test]
    fn test_query_planner_picks_index() {
        let mut mgr = IndexManager::new();
        mgr.add_index(Box::new(HashIndex::new("idx_country", "country")));
        mgr.add_index(Box::new(OrderedIndex::new("idx_age", "age")));

        // Exact match on country should pick IndexLookup
        let q1 = parse_nql("FIND users WHERE country == \"IN\"").unwrap();
        if let Statement::Find {
            filter,
            sort,
            limit,
            offset,
            ..
        } = q1
        {
            let plan = QueryPlanner::plan(filter, sort, limit, offset, &mgr);
            assert!(
                matches!(plan, QueryPlan::IndexLookup { ref index_name, .. } if index_name == "idx_country")
            );
        }

        // Range query on age should pick IndexRangeScan
        let q2 = parse_nql("FIND users WHERE age >= 18").unwrap();
        if let Statement::Find {
            filter,
            sort,
            limit,
            offset,
            ..
        } = q2
        {
            let plan = QueryPlanner::plan(filter, sort, limit, offset, &mgr);
            assert!(
                matches!(plan, QueryPlan::IndexRangeScan { ref index_name, .. } if index_name == "idx_age")
            );
        }
    }
}
