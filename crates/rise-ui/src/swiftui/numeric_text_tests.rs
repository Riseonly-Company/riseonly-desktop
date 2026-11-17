use super::numeric_text::{NumericColumn, numeric_columns};

fn column(old: Option<char>, new: Option<char>) -> NumericColumn {
    NumericColumn { old, new }
}

#[test]
fn columns_align_on_the_trailing_character() {
    assert_eq!(
        numeric_columns("9", "10"),
        vec![column(None, Some('1')), column(Some('9'), Some('0'))]
    );
    assert_eq!(
        numeric_columns("10", "9"),
        vec![column(Some('1'), None), column(Some('0'), Some('9'))]
    );
}

#[test]
fn unchanged_characters_stay_equal() {
    assert_eq!(
        numeric_columns("120", "125"),
        vec![
            column(Some('1'), Some('1')),
            column(Some('2'), Some('2')),
            column(Some('0'), Some('5')),
        ]
    );
}

#[test]
fn empty_sides_give_one_sided_columns() {
    assert_eq!(numeric_columns("", ""), Vec::<NumericColumn>::new());
    assert_eq!(numeric_columns("", "7"), vec![column(None, Some('7'))]);
}
