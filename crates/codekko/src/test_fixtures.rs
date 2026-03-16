use crate::runtime::StdioProcessSpec;

pub(crate) fn python_inline_process(script: &str) -> StdioProcessSpec {
    let mut spec = StdioProcessSpec::new("python3");
    spec.args = vec!["-u".to_owned(), "-c".to_owned(), script.to_owned()];
    spec
}
