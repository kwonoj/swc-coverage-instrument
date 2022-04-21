use swc_plugin::{ast::*, syntax_pos::DUMMY_SP, utils::take::Take};

use super::create_assignment_stmt::create_assignment_stmt;

pub fn create_global_stmt_template(coverage_global_scope: &str) -> (Ident, Stmt) {
    // var global = new Function("return $global_coverage_scope")();
    let expr = Expr::New(NewExpr {
        callee: Box::new(Expr::Ident(Ident::new("Function".into(), DUMMY_SP))),
        args: Some(vec![ExprOrSpread {
            spread: None,
            expr: Box::new(Expr::Lit(Lit::Str(Str {
                value: format!("return {}", coverage_global_scope).into(),
                ..Str::dummy()
            }))),
        }]),
        ..NewExpr::dummy()
    });

    create_assignment_stmt(
        "global",
        Expr::Call(CallExpr {
            callee: Callee::Expr(Box::new(Expr::Paren(ParenExpr {
                span: DUMMY_SP,
                expr: Box::new(expr),
            }))),
            ..CallExpr::dummy()
        }),
    )
}
