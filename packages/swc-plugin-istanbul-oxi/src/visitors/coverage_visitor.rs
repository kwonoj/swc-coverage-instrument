// TODO: remove
#![allow(unused)]
use istanbul_oxi_instrument::{BranchType, SourceCoverage};
use once_cell::sync::Lazy;
use regex::Regex as Regexp;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};
use swc_plugin::{
    ast::*,
    comments::{Comment, CommentKind, Comments, PluginCommentsProxy},
    source_map::PluginSourceMapProxy,
    syntax_pos::{Span, DUMMY_SP},
    utils::take::Take,
};
use tracing::instrument;

use crate::{
    constants::idents::*,
    create_coverage_visitor, insert_counter_helper, insert_logical_expr_helper,
    insert_stmt_counter,
    instrument::create_increase_expression_expr,
    template::{
        create_coverage_fn_decl::create_coverage_fn_decl,
        create_global_stmt_template::create_global_stmt_template,
    },
    utils::{
        hint_comments::{lookup_hint_comments, should_ignore_file},
        lookup_range::{get_expr_span, get_range_from_span, get_stmt_span},
        node::Node,
        visitor_macros::UnknownReserved,
    },
    visit_mut_coverage, visit_mut_prepend_statement_counter, InstrumentOptions,
};

use super::stmt_like_visitor::StmtVisitor;

create_coverage_visitor!(CoverageVisitor {
    file_path: String,
    attrs: UnknownReserved,
    next_ignore: Option<UnknownReserved>,
    ignore_class_method: UnknownReserved,
    types: UnknownReserved,
    source_mapping_url: Option<UnknownReserved>,
});

impl<'a> CoverageVisitor<'a> {
    insert_logical_expr_helper!();
    insert_counter_helper!();
    insert_stmt_counter!();

    fn get_var_name_hash(name: &str) -> String {
        let mut s = DefaultHasher::new();
        name.hash(&mut s);
        return format!("cov_{}", s.finish());
    }

    /// Not implemented.
    /// TODO: is this required?
    fn is_instrumented_already(&self) -> bool {
        return false;
    }

    fn on_exit_transform(&mut self, module_items: &mut Vec<ModuleItem>) {
        self.cov.freeze();

        //TODO: option: global coverage variable scope. (optional, default `this`)
        let coverage_global_scope = "this";
        //TODO: option: use an evaluated function to find coverageGlobalScope.
        let coverage_global_scope_func = true;

        let gv_template = if coverage_global_scope_func {
            // TODO: path.scope.getBinding('Function')
            let is_function_binding_scope = false;

            if is_function_binding_scope {
                /*
                gvTemplate = globalTemplateAlteredFunction({
                    GLOBAL_COVERAGE_SCOPE: T.stringLiteral(
                        'return ' + opts.coverageGlobalScope
                    )
                });
                 */
                unimplemented!("");
            } else {
                create_global_stmt_template(coverage_global_scope)
            }
        } else {
            unimplemented!("");
            /*
            gvTemplate = globalTemplateVariable({
                GLOBAL_COVERAGE_SCOPE: opts.coverageGlobalScope
            });
            */
        };

        let coverage_template = create_coverage_fn_decl(
            &self.instrument_options.coverage_variable,
            gv_template,
            &self.cov_fn_ident,
            &self.file_path,
            self.cov.as_ref(),
        );

        // explicitly call this.varName to ensure coverage is always initialized
        let m = ModuleItem::Stmt(Stmt::Expr(ExprStmt {
            span: DUMMY_SP,
            expr: Box::new(Expr::Call(CallExpr {
                callee: Callee::Expr(Box::new(Expr::Ident(self.cov_fn_ident.clone()))),
                ..CallExpr::dummy()
            })),
        }));

        // prepend template to the top of the code
        module_items.insert(0, coverage_template);
        module_items.insert(1, m);
    }
}

impl VisitMut for CoverageVisitor<'_> {
    visit_mut_coverage!();

    #[instrument(skip_all, fields(node = %self.print_node()))]
    fn visit_mut_program(&mut self, program: &mut Program) {
        self.nodes.push(Node::Program);
        if should_ignore_file(&self.comments, program) {
            return;
        }

        if self.is_instrumented_already() {
            return;
        }

        program.visit_mut_children_with(self);

        let span = match &program {
            Program::Module(m) => m.span,
            Program::Script(s) => s.span,
        };

        let coverage_data_json_str = serde_json::to_string(self.cov.as_ref())
            .expect("Should able to serialize coverage data");

        // Append coverage data as stringified JSON comments at the bottom of transformed code.
        // Currently plugin does not have way to pass any other data to the host except transformed program.
        // This attaches arbitary data to the transformed code itself to retrieve it.
        self.comments.add_trailing(
            span.hi,
            Comment {
                kind: CommentKind::Block,
                span: DUMMY_SP,
                text: format!("__coverage_data_json_comment__::{}", coverage_data_json_str).into(),
            },
        );
        self.nodes.pop();
    }

    #[instrument(skip_all, fields(node = %self.print_node()))]
    fn visit_mut_module_items(&mut self, items: &mut Vec<ModuleItem>) {
        if self.is_instrumented_already() {
            return;
        }

        self.nodes.push(Node::ModuleItems);
        let mut new_items = vec![];
        for mut item in items.drain(..) {
            item.visit_mut_children_with(self);

            if self.before.len() > 0 {
                new_items.extend(self.before.drain(..).map(|v| ModuleItem::Stmt(v)));
            }
            new_items.push(item);
        }
        *items = new_items;
        self.nodes.pop();

        self.on_exit_transform(items);
    }

    // ArrowFunctionExpression: entries(convertArrowExpression, coverFunction),
    #[instrument(skip_all, fields(node = %self.print_node()))]
    fn visit_mut_arrow_expr(&mut self, arrow_expr: &mut ArrowExpr) {
        self.nodes.push(Node::ArrowExpr);
        arrow_expr.visit_mut_children_with(self);
        self.nodes.pop();
    }

    // AssignmentPattern: entries(coverAssignmentPattern),
    #[instrument(skip_all, fields(node = %self.print_node()))]
    fn visit_mut_assign_pat(&mut self, assign_pat: &mut AssignPat) {
        self.nodes.push(Node::AssignPat);
        assign_pat.visit_mut_children_with(self);
        self.nodes.pop();
    }

    // ExportDefaultDeclaration: entries(), // ignore processing only
    #[instrument(skip_all, fields(node = %self.print_node()))]
    fn visit_mut_export_default_decl(&mut self, export_default_decl: &mut ExportDefaultDecl) {
        self.nodes.push(Node::ExportDefaultDecl);
        // noop
        export_default_decl.visit_mut_children_with(self);
        self.nodes.pop();
    }

    // ExportNamedDeclaration: entries(), // ignore processing only
    #[instrument(skip_all, fields(node = %self.print_node()))]
    fn visit_mut_export_decl(&mut self, export_named_decl: &mut ExportDecl) {
        self.nodes.push(Node::ExportDecl);
        // noop
        export_named_decl.visit_mut_children_with(self);
        self.nodes.pop();
    }

    // ClassMethod: entries(coverFunction),
    #[instrument(skip_all, fields(node = %self.print_node()))]
    fn visit_mut_class_method(&mut self, class_method: &mut ClassMethod) {
        self.nodes.push(Node::ClassMethod);
        class_method.visit_mut_children_with(self);
        self.nodes.pop();
    }

    // ClassDeclaration: entries(parenthesizedExpressionProp('superClass')),
    #[instrument(skip_all, fields(node = %self.print_node()))]
    fn visit_mut_class_decl(&mut self, class_decl: &mut ClassDecl) {
        self.nodes.push(Node::ClassDecl);
        class_decl.visit_mut_children_with(self);
        self.nodes.pop();
    }

    // ClassProperty: entries(coverClassPropDeclarator),
    #[instrument(skip_all, fields(node = %self.print_node()))]
    fn visit_mut_class_prop(&mut self, class_prop: &mut ClassProp) {
        self.nodes.push(Node::ClassProp);
        class_prop.visit_mut_children_with(self);
        self.nodes.pop();
    }

    // ClassPrivateProperty: entries(coverClassPropDeclarator),
    #[instrument(skip_all, fields(node = %self.print_node()))]
    fn visit_mut_private_prop(&mut self, private_prop: &mut PrivateProp) {
        self.nodes.push(Node::PrivateProp);
        private_prop.visit_mut_children_with(self);
        self.nodes.pop();
    }

    // BreakStatement: entries(coverStatement),
    #[instrument(skip_all, fields(node = %self.print_node()))]
    fn visit_mut_break_stmt(&mut self, break_stmt: &mut BreakStmt) {
        self.nodes.push(Node::BreakStmt);
        break_stmt.visit_mut_children_with(self);
        self.nodes.pop();
    }

    // BreakStatement: entries(coverStatement),
    #[instrument(skip_all, fields(node = %self.print_node()))]
    fn visit_mut_continue_stmt(&mut self, continue_stmt: &mut ContinueStmt) {
        self.nodes.push(Node::ContinueStmt);
        continue_stmt.visit_mut_children_with(self);
        self.nodes.pop();
    }

    // DebuggerStatement: entries(coverStatement),
    #[instrument(skip_all, fields(node = %self.print_node()))]
    fn visit_mut_debugger_stmt(&mut self, debugger_stmt: &mut DebuggerStmt) {
        self.nodes.push(Node::DebuggerStmt);
        debugger_stmt.visit_mut_children_with(self);
        self.nodes.pop();
    }

    // ThrowStatement: entries(coverStatement),
    #[instrument(skip_all, fields(node = %self.print_node()))]
    fn visit_mut_throw_stmt(&mut self, throw_stmt: &mut ThrowStmt) {
        self.nodes.push(Node::ThrowStmt);
        throw_stmt.visit_mut_children_with(self);
        self.nodes.pop();
    }

    // TryStatement: entries(coverStatement),
    #[instrument(skip_all, fields(node = %self.print_node()))]
    fn visit_mut_try_stmt(&mut self, try_stmt: &mut TryStmt) {
        self.nodes.push(Node::TryStmt);
        try_stmt.visit_mut_children_with(self);
        self.nodes.pop();
    }

    /*
    // ForStatement: entries(blockProp('body'), coverStatement),
    #[instrument(skip_all, fields(node = %self.print_node()))]
    fn visit_mut_for_stmt(&mut self, for_stmt: &mut ForStmt) {
        self.nodes.push(Node::ForStmt);

        self.mark_prepend_stmt_counter(&for_stmt.span);

        for_stmt.visit_mut_children_with(self);

        let mut body = *for_stmt.body.take();
        // if for stmt body is not block, wrap it before insert statement counter
        let body = if let Stmt::Block(mut body) = body {
            //self.visit_mut_stmts(&mut body.stmts);
            body
        } else {
            let mut stmts = vec![body];
            self.insert_stmts_counter(&mut stmts);

            BlockStmt {
                span: DUMMY_SP,
                stmts,
            }
        };
        for_stmt.body = Box::new(Stmt::Block(body));

        self.nodes.pop();
    } */

    // ForInStatement: entries(blockProp('body'), coverStatement),
    #[instrument(skip_all, fields(node = %self.print_node()))]
    fn visit_mut_for_in_stmt(&mut self, for_in_stmt: &mut ForInStmt) {
        self.nodes.push(Node::ForInStmt);

        self.mark_prepend_stmt_counter(&for_in_stmt.span);

        for_in_stmt.visit_mut_children_with(self);

        let mut body = *for_in_stmt.body.take();
        // if for stmt body is not block, wrap it before insert statement counter
        let body = if let Stmt::Block(mut body) = body {
            //self.visit_mut_stmts(&mut body.stmts);
            body
        } else {
            let mut stmts = vec![body];
            self.insert_stmts_counter(&mut stmts);

            BlockStmt {
                span: DUMMY_SP,
                stmts,
            }
        };
        for_in_stmt.body = Box::new(Stmt::Block(body));

        self.nodes.pop();
    }

    // ForOfStatement: entries(blockProp('body'), coverStatement),
    #[instrument(skip_all, fields(node = %self.print_node()))]
    fn visit_mut_for_of_stmt(&mut self, for_of_stmt: &mut ForOfStmt) {
        self.nodes.push(Node::ForOfStmt);
        self.mark_prepend_stmt_counter(&for_of_stmt.span);

        for_of_stmt.visit_mut_children_with(self);

        let mut body = *for_of_stmt.body.take();
        // if for stmt body is not block, wrap it before insert statement counter
        let body = if let Stmt::Block(mut body) = body {
            //self.visit_mut_stmts(&mut body.stmts);
            body
        } else {
            let mut stmts = vec![body];
            self.insert_stmts_counter(&mut stmts);

            BlockStmt {
                span: DUMMY_SP,
                stmts,
            }
        };
        for_of_stmt.body = Box::new(Stmt::Block(body));
        self.nodes.pop();
    }

    // WhileStatement: entries(blockProp('body'), coverStatement),
    #[instrument(skip_all, fields(node = %self.print_node()))]
    fn visit_mut_while_stmt(&mut self, while_stmt: &mut WhileStmt) {
        self.nodes.push(Node::WhileStmt);
        while_stmt.visit_mut_children_with(self);
        self.nodes.pop();
    }

    // DoWhileStatement: entries(blockProp('body'), coverStatement),
    #[instrument(skip_all, fields(node = %self.print_node()))]
    fn visit_mut_do_while_stmt(&mut self, do_while_stmt: &mut DoWhileStmt) {
        self.nodes.push(Node::DoWhileStmt);
        do_while_stmt.visit_mut_children_with(self);
        self.nodes.pop();
    }

    // SwitchStatement: entries(createSwitchBranch, coverStatement),
    #[instrument(skip_all, fields(node = %self.print_node()))]
    fn visit_mut_switch_stmt(&mut self, switch_stmt: &mut SwitchStmt) {
        self.nodes.push(Node::SwitchStmt);
        switch_stmt.visit_mut_children_with(self);
        self.nodes.pop();
    }

    // SwitchCase: entries(coverSwitchCase),
    #[instrument(skip_all, fields(node = %self.print_node()))]
    fn visit_mut_switch_case(&mut self, switch_case: &mut SwitchCase) {
        self.nodes.push(Node::SwitchCase);
        switch_case.visit_mut_children_with(self);
        self.nodes.pop();
    }

    // WithStatement: entries(blockProp('body'), coverStatement),
    #[instrument(skip_all, fields(node = %self.print_node()))]
    fn visit_mut_with_stmt(&mut self, with_stmt: &mut WithStmt) {
        self.nodes.push(Node::WithStmt);
        with_stmt.visit_mut_children_with(self);
        self.nodes.pop();
    }

    // ConditionalExpression: entries(coverTernary),
    #[instrument(skip_all, fields(node = %self.print_node()))]
    fn visit_mut_cond_expr(&mut self, cond_expr: &mut CondExpr) {
        let (old, ignore_current) = self.on_enter(cond_expr);

        if ignore_current {
            cond_expr.visit_mut_children_with(self);
            self.on_exit(old);
            return;
        }

        let range = get_range_from_span(self.source_map, &cond_expr.span);
        let branch = self.cov.new_branch(BranchType::CondExpr, &range, false);

        let c_hint = lookup_hint_comments(&self.comments, get_expr_span(&*cond_expr.cons));
        let a_hint = lookup_hint_comments(&self.comments, get_expr_span(&*cond_expr.alt));

        if c_hint.as_deref() != Some("next") {
            // TODO: do we need this?
            // cond_expr.cons.visit_mut_children_with(self);

            // replace consequence to the paren for increase expr + expr itself
            self.replace_expr_with_branch_counter(&mut *cond_expr.cons, branch);
        }

        if a_hint.as_deref() != Some("next") {
            // TODO: do we need this?
            // cond_expr.alt.visit_mut_children_with(self);

            // replace consequence to the paren for increase expr + expr itself
            self.replace_expr_with_branch_counter(&mut *cond_expr.alt, branch);
        }

        cond_expr.visit_mut_children_with(self);
        self.on_exit(old);
    }

    // ObjectMethod: entries(coverFunction),
    #[instrument(skip_all, fields(node = %self.print_node()))]
    fn visit_mut_method_prop(&mut self, method_prop: &mut MethodProp) {
        self.nodes.push(Node::MethodProp);
        method_prop.visit_mut_children_with(self);
        self.nodes.pop();
        // ObjectMethodKind::Method,
    }

    // ObjectMethod: entries(coverFunction),
    #[instrument(skip_all, fields(node = %self.print_node()))]
    fn visit_mut_getter_prop(&mut self, getter_prop: &mut GetterProp) {
        self.nodes.push(Node::GetterProp);
        getter_prop.visit_mut_children_with(self);
        self.nodes.pop();
        // ObjectMethodKind::Get,
    }

    // ObjectMethod: entries(coverFunction),
    #[instrument(skip_all, fields(node = %self.print_node()))]
    fn visit_mut_setter_prop(&mut self, setter_prop: &mut SetterProp) {
        self.nodes.push(Node::SetterProp);
        setter_prop.visit_mut_children_with(self);
        self.nodes.pop();
        //ObjectMethodKind::Set,
    }
}
