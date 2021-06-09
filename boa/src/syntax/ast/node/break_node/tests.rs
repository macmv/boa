use crate::{
    exec::{Executable, InterpreterState},
    parse,
    syntax::ast::node::Break,
    Context,
};

#[test]
fn check_post_state() {
    let mut context = Context::new();

    let brk: Break = Break::new("label");

    brk.run(&mut context).unwrap();

    assert_eq!(
        context.executor().get_current_state(),
        &InterpreterState::Break(Some("label".into()))
    );
}

#[test]
fn fmt() {
    // Blocks do not store their label, so we cannot test with
    // the outer block having a label.
    //
    // TODO: Once block labels are implemented, this test should
    // include them:
    //
    // ```
    // outer: {
    //     while (true) {
    //         break outer;
    //     }
    //     skipped_call();
    // }
    // ```
    let scenario = r#"
        {
            while (true) {
                break outer;
            }
            skipped_call();
        }
        while (true) {
            break;
        }
        "#[1..] // Remove the preceding newline
        .lines()
        .map(|l| &l[8..]) // Remove preceding whitespace from each line
        .collect::<Vec<&'static str>>()
        .join("\n");
    assert_eq!(format!("{}", parse(&scenario, false).unwrap()), scenario);
}
