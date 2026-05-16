use starlark::environment::Globals;
use starlark::environment::Module;
use starlark::eval::Evaluator;
use starlark::syntax::AstModule;
use starlark::syntax::Dialect;
use starlark::values::Value;

fn main() {
    let content = r#"
def hello():
   return "hello"
hello() + " world!"
"#;

    // We first parse the content, giving a filename and the Starlark
    // `Dialect` we'd like to use (we pick standard).
    let ast: AstModule = AstModule::parse(
        "hello_world.star",
        content.to_owned(),
        &Dialect::Standard,
    )?;

    // We create a `Globals`, defining the standard library functions available.
    // The `standard` function uses those defined in the Starlark specification.
    let globals: Globals = Globals::standard();

    // We create a `Module`, which stores the global variables for our calculation.
    let module: Module = Module::new();

    // We create an evaluator, which controls how evaluation occurs.
    let mut eval: Evaluator = Evaluator::new(&module);

    // And finally we evaluate the code using the evaluator.
    let res: Value = eval.eval_module(ast, &globals)?;
    assert_eq!(res.unpack_str(), Some("hello world!"));
}
