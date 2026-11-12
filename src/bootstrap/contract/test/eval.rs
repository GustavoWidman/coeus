use crate::bootstrap::contract::Failure;

const THROWN: &str = "error:
       … while calling anonymous lambda
         at «string»:1:1:
            1| c: c.enroll (builtins.fromJSON \"{}\")
             | ^

       … while calling the 'throw' builtin
         at /repo/.coeus/bootstrap.nix:290:7:
          289|     else
          290|       throw \"coeus bootstrap: no boot loader this repository knows (null); write the machine's boot.nix by hand\";
             |       ^
          291|

       error: coeus bootstrap: no boot loader this repository knows (null); write the machine's boot.nix by hand
";

#[test]
fn a_throw_is_the_contracts_own_words() {
    assert_eq!(
        Failure::read(THROWN),
        Failure::Said(
            "coeus bootstrap: no boot loader this repository knows (null); write the machine's boot.nix by hand".into()
        )
    );

    let lines = "error:
       … while calling the 'throw' builtin
         at /repo/.coeus/bootstrap.nix:1:23:
            1| x
             | ^

       error: first line
       second line
";
    assert_eq!(
        Failure::read(lines),
        Failure::Said("first line\nsecond line".into())
    );
}

#[test]
fn an_abort_is_the_contracts_own_words_too() {
    let aborted = "error:
       … while calling the 'abort' builtin
         at /repo/.coeus/bootstrap.nix:1:100:
            1| x
             | ^

       error: evaluation aborted with the following error message: 'no such machine'
";
    assert_eq!(
        Failure::read(aborted),
        Failure::Said("no such machine".into())
    );
}

#[test]
fn a_broken_contract_says_where() {
    let missing = "error:
       … while calling 'h'
         at /repo/.coeus/bootstrap.nix:1:126:
            1| x
             | ^

       error: attribute 'missing' missing
       at /repo/.coeus/bootstrap.nix:1:129:
            1| x
             | ^
";
    let failure = Failure::read(missing);
    assert_eq!(
        failure,
        Failure::Broken {
            error: "attribute 'missing' missing".into(),
            at: Some("/repo/.coeus/bootstrap.nix:1:129".into()),
        }
    );
    assert_eq!(
        failure.to_string(),
        "the contract failed at /repo/.coeus/bootstrap.nix:1:129: attribute 'missing' missing"
    );

    let syntax = "error: syntax error, unexpected end of file
       at /repo/.coeus/bootstrap.nix:2:1:
            1| {
";
    assert_eq!(
        Failure::read(syntax),
        Failure::Broken {
            error: "syntax error, unexpected end of file".into(),
            at: Some("/repo/.coeus/bootstrap.nix:2:1".into()),
        }
    );
    assert_eq!(
        Failure::read("Segmentation fault\n"),
        Failure::Broken {
            error: "Segmentation fault".into(),
            at: None
        }
    );
}
