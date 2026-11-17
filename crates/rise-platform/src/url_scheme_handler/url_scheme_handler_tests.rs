use rise_core::foundation::foundation_url::Url;

use super::url_scheme_handler::{
    UrlSchemeHandlerSource, UrlSchemeHandlerUnavailable, can_open_url_with,
};

struct Answers(Result<bool, UrlSchemeHandlerUnavailable>);

impl UrlSchemeHandlerSource for Answers {
    fn has_handler(&self, _url: &Url) -> Result<bool, UrlSchemeHandlerUnavailable> {
        self.0.clone()
    }
}

struct DefaultBody;

impl UrlSchemeHandlerSource for DefaultBody {}

fn telegram() -> Url {
    let raw = "tg://resolve?domain=a";
    let Some(url) = Url::from_string(raw) else {
        panic!("{raw} is not a URL");
    };
    url
}

#[test]
fn the_answer_of_the_os_is_the_answer() {
    assert!(can_open_url_with(&Answers(Ok(true)), &telegram()));
    assert!(!can_open_url_with(&Answers(Ok(false)), &telegram()));
}

#[test]
fn a_branch_not_written_reads_as_no_handler() {
    assert!(matches!(
        DefaultBody.has_handler(&telegram()),
        Err(UrlSchemeHandlerUnavailable::BranchNotWritten { .. })
    ));
    assert!(!can_open_url_with(&DefaultBody, &telegram()));
}
