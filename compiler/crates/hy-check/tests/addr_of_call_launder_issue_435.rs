use hy_check::{check_file, Verdict};

fn check(src: &str) -> hy_check::CheckResult {
    let ast = hy_syntax::parse_file("test.hy", src).expect("test source must parse");
    check_file(&ast)
}

#[test]
fn issue_435_addr_of_laundered_through_call_freezes_on_move_of_owner() {
    let src = r#"
struct holder {
    m map[string]int
}

fn keep(h *holder) *holder {
    return h
}

fn worker(h holder) {
    h.m["worker"] = 1
}

fn main() {
    var h holder
    h.m = map[string]int{"init": 0}
    p := keep(&h)
    task worker(move h)
    p.m["main"]++
}
"#;
    let r = check(src);
    assert_eq!(r.verdict, Verdict::Reject, "expected Reject, got {:?}", r.diagnostics);
    assert!(
        r.diagnostics.iter().any(|d| d.code == "HYB-E3003"),
        "expected HYB-E3003 for post-move use of call-laundered borrow `p`, got {:?}",
        r.diagnostics
    );
}

#[test]
fn issue_435_value_struct_addr_of_laundered_through_call_freezes_both_directions() {
    // Moving `p` must freeze `c` even when `c` is a value struct (`shape_may_share == false`).
    let src_move_ptr = r#"
struct counter {
    n int
}

fn keep(p *counter) *counter {
    return p
}

fn worker(p *counter) {
    p.n = 1
}

fn main() {
    var c counter
    c.n = 0
    p := keep(&c)
    task worker(move p)
    c.n = 99
}
"#;
    let r1 = check(src_move_ptr);
    assert_eq!(r1.verdict, Verdict::Reject, "expected Reject, got {:?}", r1.diagnostics);
    assert!(
        r1.diagnostics.iter().any(|d| d.code == "HYB-E3003"),
        "expected HYB-E3003 when mutating `c` after `move p`, got {:?}",
        r1.diagnostics
    );

    // Rebind form (`p = keep(&c)`) and multi-return form (`p, ok := keep_pair(&c)`).
    let src_rebind_and_multi = r#"
struct counter {
    n int
}

fn keep(p *counter) *counter {
    return p
}

fn keep_pair(p *counter) (*counter, bool) {
    return p, true
}

fn worker(p *counter) {
    p.n = 1
}

fn main() {
    var a counter
    var b counter
    var p *counter
    p = keep(&a)
    task worker(move p)
    a.n = 10

    p2, ok := keep_pair(&b)
    _ = ok
    task worker(move p2)
    b.n = 20
}
"#;
    let r2 = check(src_rebind_and_multi);
    assert_eq!(r2.verdict, Verdict::Reject, "expected Reject, got {:?}", r2.diagnostics);
    let e3003_count = r2.diagnostics.iter().filter(|d| d.code == "HYB-E3003").count();
    assert!(
        e3003_count >= 2,
        "expected at least 2 HYB-E3003 diagnostics for rebind and multi-return, got {:?}",
        r2.diagnostics
    );
}

#[test]
fn issue_435_non_owned_and_const_value_struct_laundered_through_call_rejects() {
    let src_param = r#"
struct counter {
    n int
}

fn keep(p *counter) *counter {
    return p
}

fn worker(p *counter) {
    p.n = 1
}

fn outer(c counter) {
    p := keep(&c)
    task worker(move p)
}
"#;
    let r_param = check(src_param);
    assert_eq!(r_param.verdict, Verdict::Reject, "expected Reject, got {:?}", r_param.diagnostics);
    assert!(
        r_param.diagnostics.iter().any(|d| d.code == "HYB-E3004"),
        "expected HYB-E3004 when moving pointer laundered from non-owned parameter `c`, got {:?}",
        r_param.diagnostics
    );

    let src_const = r#"
struct counter {
    n int
}

const shared = counter{n: 0}

fn keep(p *counter) *counter {
    return p
}

fn main() {
    p := keep(&shared)
    p.n = 99
}
"#;
    let r_const = check(src_const);
    assert_eq!(r_const.verdict, Verdict::Reject, "expected Reject, got {:?}", r_const.diagnostics);
    assert!(
        r_const.diagnostics.iter().any(|d| d.code == "HYB-E3006"),
        "expected HYB-E3006 when writing through pointer laundered from `const shared`, got {:?}",
        r_const.diagnostics
    );
}

#[test]
fn issue_435_return_of_call_laundered_borrow_rejects_loan_escape_e3104() {
    let src_bound = r#"
struct holder {
    m map[string]int
}

fn keep(h *holder) *holder {
    return h
}

fn escape_bound() *holder {
    var h holder
    h.m = map[string]int{"init": 0}
    p := keep(&h)
    return p
}
"#;
    let r1 = check(src_bound);
    assert_eq!(r1.verdict, Verdict::Reject, "expected Reject, got {:?}", r1.diagnostics);
    assert!(
        r1.diagnostics.iter().any(|d| d.code == "HYB-E3104"),
        "expected HYB-E3104 for returning bound call-laundered borrow `p`, got {:?}",
        r1.diagnostics
    );

    let src_direct = r#"
struct holder {
    m map[string]int
}

fn keep(h *holder) *holder {
    return h
}

fn escape_direct() *holder {
    var h holder
    h.m = map[string]int{"init": 0}
    return keep(&h)
}
"#;
    let r2 = check(src_direct);
    assert_eq!(r2.verdict, Verdict::Reject, "expected Reject, got {:?}", r2.diagnostics);
    assert!(
        r2.diagnostics.iter().any(|d| d.code == "HYB-E3104"),
        "expected HYB-E3104 for directly returning `keep(&h)`, got {:?}",
        r2.diagnostics
    );
}

#[test]
fn issue_435_scalar_return_call_with_addr_of_accepts_control() {
    let src = r#"
import "fmt"

struct holder {
    m map[string]int
}

fn read(h *holder) int {
    return h.m["init"]
}

fn worker(h holder) {
    h.m["worker"] = 1
}

fn main() {
    var h holder
    h.m = map[string]int{"init": 7}
    n := read(&h)
    task worker(move h)
    fmt.println(n)
}
"#;
    let r = check(src);
    assert_eq!(
        r.verdict,
        Verdict::Accept,
        "scalar-returning call `read(&h)` must not create a borrow alias on `n`, got {:?}",
        r.diagnostics
    );
    assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
}
