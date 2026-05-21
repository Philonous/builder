use std::borrow::Cow;

use crate::parser::parser;
use std::process;

pub struct Ident<'a>(pub Cow<'a, str>);

enum Lit<'a> {
    String(Cow<'a, str>),
}

enum Expr<'a> {
    Lit(Lit<'a>),
    FunApp(Ident<'a>),
}

enum Statement<'a> {
    Command(Vec<Cow<'a, str>>),
}

impl<'a> parser::Statement<'a> {
    fn to_statement(self) -> Statement<'a> {
        match self {
            parser::Statement::Expr(parser::Expr::Command(cmd)) => {
                Statement::Command(cmd.resolve_to_strings()?)
            }
            _ => todo!(),
        }
    }
}

pub struct Target<'a> {
    pub typ: Ident<'a>,
    pub name: Cow<'a, str>,
    commands: Vec<Statement<'a>>,
}

// impl<'a> Target<'a> {
//     fn exec(&self) {
//         for s in self.commands {
//         }
//     }
// }

impl<'a> parser::Decl<'a> {
    fn to_target(self) -> Target<'a> {
        match self.header {
            parser::DeclHeader::AtDecl(ident) => Target {
                typ: Ident(Cow::Owned("file".to_owned())),
                name: Cow::Borrowed(ident.0),
                commands: self
                    .definition
                    .into_iter()
                    .map(|def| def.to_statement())
                    .collect(),
            },
            _ => todo!(),
        }
    }
}

impl<'a> parser::Ast<'a> {
    pub fn to_targets(self) -> Vec<Target<'a>> {
        self.decls
            .into_iter()
            .map(|decl| decl.to_target())
            .collect()
    }
}
