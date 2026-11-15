use super::{Lexer, Limits, Options, Token};

/// As a mail reader sets it up.
const MAIL: Options<'static> =
    Options { limits: Limits { max_attrs: 32, max_tag_name: 24 }, drop: &["script", "style", "title", "iframe"], raw_text: &["textarea", "xmp"] };

fn show<'a>(lexer: impl Iterator<Item = Token<'a>>) -> Vec<String> {
    lexer
        .map(|t| match t {
            Token::Text(t) => format!("T:{t}"),
            Token::Start(tag) => {
                let attrs: Vec<String> = tag.attrs.iter().map(|(n, v)| format!("{n}={v}")).collect();
                if attrs.is_empty() { format!("<{}>", tag.name) } else { format!("<{} {}>", tag.name, attrs.join(" ")) }
            }
            Token::End(n) => format!("</{n}>"),
        })
        .collect()
}

fn tokens(html: &str) -> Vec<String> {
    show(Lexer::with(html, MAIL))
}

#[test]
fn text_and_tags() {
    assert_eq!(tokens("Hi <B>there</b>!"), ["T:Hi ", "<b>", "T:there", "</b>", "T:!"]);
    assert_eq!(tokens(""), Vec::<String>::new());
    assert_eq!(tokens("plain"), ["T:plain"]);
    assert_eq!(tokens("<br/><hr />"), ["<br>", "<hr>"]);
}

#[test]
fn attributes_in_every_quoting_style() {
    assert_eq!(tokens(r#"<a href="x y" title='it"s' id=z data-k>"#), [r#"<a href=x y title=it"s id=z data-k=>"#]);
    assert_eq!(tokens("<a href = \"u\"\n\tclass = c >t"), ["<a href=u class=c>", "T:t"]);
    assert_eq!(tokens("<img src=a.png/>"), ["<img src=a.png/>"]);
    assert_eq!(tokens("<p =x=y>"), ["<p =x=y>"]);
    // A `>` inside a quoted value does not end the tag.
    assert_eq!(tokens(r#"<a title="a>b">t</a>"#), ["<a title=a>b>", "T:t", "</a>"]);
}

#[test]
fn attribute_values_are_decoded_on_demand() {
    let mut t = Lexer::new(r#"<a HREF="?a=1&amp;b=2&copy=3" href="second" Checked>"#);
    let Some(Token::Start(tag)) = t.next() else { panic!("no tag") };
    assert_eq!(tag.attr("href").as_deref(), Some("?a=1&b=2&copy=3"));
    assert_eq!(tag.raw_attr("href"), Some("?a=1&amp;b=2&copy=3"));
    assert_eq!(tag.attr("missing"), None);
    assert!(tag.has("checked") && !tag.has("check"));
    assert_eq!(tag.attributes().map(|(n, _)| n).collect::<Vec<_>>(), ["HREF", "href", "Checked"]);
}

#[test]
fn a_tag_that_closes_itself_says_so() {
    let closes = |html: &str| match Lexer::new(html).next() {
        Some(Token::Start(tag)) => tag.self_close,
        other => panic!("{other:?}"),
    };
    for yes in ["<br/>", "<br />", "<img src=\"a\"/>", "<img src=a />", "<p\n/>"] {
        assert!(closes(yes), "{yes}");
    }
    // A `/` that ends an unquoted value, or stands anywhere but before the `>`, is not it.
    for no in ["<br>", "<img src=a.png/>", "<a / b>", "<a/ >", "<br/"] {
        assert!(!closes(no), "{no}");
    }
}

#[test]
fn a_lone_angle_bracket_is_text() {
    assert_eq!(tokens("a < b and c > d"), ["T:a < b and c > d"]);
    assert_eq!(tokens("1<2<3"), ["T:1<2<3"]);
    assert_eq!(tokens("<"), ["T:<"]);
    assert_eq!(tokens("x<"), ["T:x<"]);
    assert_eq!(tokens("<3 <- </ x"), ["T:<3 <- </ x"]);
    assert_eq!(tokens("a</>b"), ["T:a", "T:b"]);
}

#[test]
fn comments_and_declarations_vanish() {
    assert_eq!(tokens("a<!-- x -- > y -->b"), ["T:a", "T:b"]);
    assert_eq!(tokens("<!DOCTYPE html><p>x"), ["<p>", "T:x"]);
    assert_eq!(tokens("<?xml version=\"1.0\"?>x"), ["T:x"]);
    assert_eq!(tokens("a<!-->b<!--->c"), ["T:a", "T:b", "T:c"]);
    assert_eq!(tokens("<!--[if mso]><p>only word</p><![endif]-->shown"), ["T:shown"]);
    assert_eq!(tokens("<![if !mso]>shown<![endif]>"), ["T:shown"]);
    // An unfinished comment swallows the rest.
    assert_eq!(tokens("a<!-- b <p>c"), ["T:a"]);
}

#[test]
fn dropped_elements_never_leave_the_lexer() {
    assert_eq!(tokens("a<script>if (a<b) alert('</p>')</script>b"), ["T:a", "T:b"]);
    assert_eq!(tokens("a<STYLE type=text/css>p > b { color: red }</Style >b"), ["T:a", "T:b"]);
    assert_eq!(tokens("<title>Hello</title>x"), ["T:x"]);
    assert_eq!(tokens("a<script>never closed <p>x"), ["T:a"]);
    // `</scripty>` is not the end tag.
    assert_eq!(tokens("<script>x</scripty>y</script>z"), ["T:z"]);
    // Dropping is an option: by default the tags come out and the content is markup like any other.
    assert_eq!(show(Lexer::new("a<script>x<b>y</script>b")), ["T:a", "<script>", "T:x", "<b>", "T:y", "</script>", "T:b"]);
}

#[test]
fn raw_content_is_the_callers_to_take() {
    let src = "<style media=print>p > b { color: red } </p></STYLE >after<script>x";
    let mut l = Lexer::new(src);
    let Some(Token::Start(tag)) = l.next() else { panic!("no tag") };
    assert_eq!((tag.name.as_str(), tag.raw_attr("media")), ("style", Some("print")));
    assert_eq!(l.raw_content("style"), "p > b { color: red } </p>");
    assert_eq!(l.next(), Some(Token::Text("after")));
    assert!(matches!(l.next(), Some(Token::Start(t)) if t.name == "script"));
    // No end tag: to the end of the input.
    assert_eq!(l.raw_content("script"), "x");
    assert_eq!((l.next(), l.pos()), (None, src.len()));
}

#[test]
fn raw_text_content_is_text() {
    assert_eq!(tokens("<textarea><b>x</b></textarea>y"), ["<textarea>", "T:<b>x</b>", "</textarea>", "T:y"]);
    assert_eq!(tokens("<textarea></textarea>"), ["<textarea>", "</textarea>"]);
    assert_eq!(tokens("<xmp>a<p>b"), ["<xmp>", "T:a<p>b"]);
    assert_eq!(show(Lexer::new("<xmp>a<p>b")), ["<xmp>", "T:a", "<p>", "T:b"]);
}

#[test]
fn unfinished_tags_run_to_the_end() {
    assert_eq!(tokens("a<b"), ["T:a", "<b>"]);
    assert_eq!(tokens("a<a href=\"x"), ["T:a", "<a href=x>"]);
    assert_eq!(tokens("a</b"), ["T:a", "</b>"]);
    assert_eq!(tokens("a<b c"), ["T:a", "<b c=>"]);
    assert_eq!(tokens("a<b c="), ["T:a", "<b c=>"]);
}

#[test]
fn non_ascii_everywhere() {
    assert_eq!(tokens("é<b é=ü ñ>ß</b>ø"), ["T:é", "<b é=ü ñ=>", "T:ß", "</b>", "T:ø"]);
    assert_eq!(tokens("<é>"), ["T:<é>"]);
    assert_eq!(tokens("<bé>x"), ["<bé>", "T:x"]);
}

#[test]
fn long_names_and_many_attributes_are_capped() {
    let name = |n: usize, opts: Options<'static>| show(Lexer::with(&format!("<{0}>x</{0}>", "a".repeat(n)), opts));
    assert_eq!(name(25, MAIL), ["<>", "T:x", "</>"]);
    assert_eq!(name(24, MAIL).first().map(String::len), Some(26));
    assert_eq!(name(65, Options::default()), ["<>", "T:x", "</>"]);
    assert_eq!(name(64, Options::default()).first().map(String::len), Some(66));
    let many = format!("<p {}>", (0..100).map(|i| format!("a{i}=1")).collect::<Vec<_>>().join(" "));
    for (opts, kept) in [(MAIL, 32), (Options::default(), 64)] {
        let Some(Token::Start(tag)) = Lexer::with(&many, opts).next() else { panic!("no tag") };
        assert_eq!(tag.attrs.len(), kept);
    }
}

#[test]
fn hostile_runs_are_linear() {
    for s in [
        "<".repeat(300_000),
        "<a ".repeat(100_000),
        "<!--".repeat(100_000),
        "</".repeat(150_000),
        "<script>".repeat(50_000),
        "<b x=\"".repeat(50_000),
    ] {
        assert!(Lexer::with(&s, MAIL).count() <= s.len());
        assert!(Lexer::new(&s).count() <= s.len());
    }
}

proptest::proptest! {
    #[test]
    fn any_input_tokenizes_and_keeps_the_text(s in "[<>/!a-c =\"'&;é-]{0,80}") {
        for opts in [MAIL, Options::default()] {
            let total: usize = Lexer::with(&s, opts).map(|t| match t { Token::Text(t) => t.len(), _ => 0 }).sum();
            proptest::prop_assert!(total <= s.len());
        }
    }
}

#[test]
fn a_token_can_be_read_again() {
    let mut l = Lexer::new("é<b>x</b>");
    assert_eq!(l.next(), Some(Token::Text("é")));
    let at = l.pos();
    assert!(matches!(l.next(), Some(Token::Start(t)) if t.name == "b"));
    l.set_pos(at);
    assert!(matches!(l.next(), Some(Token::Start(t)) if t.name == "b"));
    // Inside a character or past the end: the next boundary.
    l.set_pos(1);
    assert!(matches!(l.next(), Some(Token::Start(t)) if t.name == "b"));
    l.set_pos(1000);
    assert_eq!(l.next(), None);
}
