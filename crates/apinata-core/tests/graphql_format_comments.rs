//! `format_query` comments: trailing and full-line comments, broken lists, blank lines and
//! one comment in each place the parser puts one.

use crate::graphql_format::assert_canonical;

#[test]
fn a_trailing_comment_stays_on_its_line() {
    assert_canonical("{ a # why\n b }", "{\n  a # why\n  b\n}\n");
}

#[test]
fn a_full_line_comment_takes_the_current_indent() {
    assert_canonical(
        "{\n# lead\na\n      # tail\n}",
        "{\n  # lead\n  a\n  # tail\n}\n",
    );
}

#[test]
fn a_comment_after_an_opening_brace_stays_beside_it() {
    assert_canonical("{ # top\n a }", "{ # top\n  a\n}\n");
}

#[test]
fn comment_trailing_whitespace_is_trimmed() {
    assert_canonical("{\n  a # c   \n}\n", "{\n  a # c\n}\n");
}

#[test]
fn top_level_comments_keep_their_definition_and_blank_lines() {
    assert_canonical(
        "# Users\nquery A { a } # end\n# B\n\nquery B { b }",
        "# Users\nquery A {\n  a\n} # end\n\n# B\n\nquery B {\n  b\n}\n",
    );
}

#[test]
fn one_blank_line_is_kept_between_selections() {
    assert_canonical(
        "{\n\n  a\n\n\n  b\n  # c\n\n  d\n\n}",
        "{\n  a\n\n  b\n  # c\n\n  d\n}\n",
    );
}

#[test]
fn a_comment_breaks_its_argument_list_one_item_per_line() {
    assert_canonical(
        "{ user(id: 1 # why\n , x: 2) { name } }",
        "{\n  user(\n    id: 1 # why\n    x: 2\n  ) {\n    name\n  }\n}\n",
    );
    assert_canonical("{ a( # c\n x: 1) }", "{\n  a( # c\n    x: 1\n  )\n}\n");
}

#[test]
fn a_comment_breaks_variable_definitions() {
    assert_canonical(
        "query Q($v: # c5\n Int, $w: Int) { a }",
        "query Q(\n  $v: # c5\n  Int\n  $w: Int\n) {\n  a\n}\n",
    );
}

#[test]
fn a_comment_in_a_list_value_breaks_it_and_its_argument_list() {
    assert_canonical(
        "{ a(l: [1, # one\n 2]) }",
        "{\n  a(\n    l: [\n      1 # one\n      2\n    ]\n  )\n}\n",
    );
}

#[test]
fn a_comment_in_an_object_value_breaks_it() {
    assert_canonical(
        "{ a(o: {k: 1 # c\n m: 2}) }",
        "{\n  a(\n    o: {\n      k: 1 # c\n      m: 2\n    }\n  )\n}\n",
    );
}

#[test]
fn a_block_string_in_a_broken_list_keeps_its_spaces() {
    assert_canonical(
        "{ a(t: \"\"\"x  \n\"\"\" # c\n) }",
        "{\n  a(\n    t: \"\"\"x  \n\"\"\" # c\n  )\n}\n",
    );
}

#[test]
fn comments_inside_a_field_after_the_alias_and_before_arguments() {
    assert_canonical(
        "{ a: # c1\n user # c2\n (id: 1) { name } }",
        "{\n  a: # c1\n  user # c2\n  (id: 1) {\n    name\n  }\n}\n",
    );
}

#[test]
fn a_comment_between_directives() {
    assert_canonical(
        "{ user(id: 1) @x # c3\n @y { name } }",
        "{\n  user(id: 1) @x # c3\n  @y {\n    name\n  }\n}\n",
    );
}

#[test]
fn a_comment_in_an_operation_before_its_variables() {
    assert_canonical(
        "query Q # c4\n ($v: Int) { a }",
        "query Q # c4\n($v: Int) {\n  a\n}\n",
    );
}

#[test]
fn a_lone_carriage_return_ends_a_line() {
    assert_canonical("# a\r# b\n{ a }", "# a\n# b\n{\n  a\n}\n");
    assert_canonical("{ a\r# c\r b }", "{\n  a\n  # c\n  b\n}\n");
}
