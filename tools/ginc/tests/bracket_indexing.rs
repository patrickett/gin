mod common;
use common::run_in_profiles;
use test_fixtures::TempPackage;

#[test]
fn bracket_indexing_and_inline_copy_preserve_values() {
    let package = TempPackage::new("bracket_indexing");
    let input = package.write(
        "main.gin",
        r#"Copy has
    copy(self ref) Self: return (bits is in Self: self) as Self
Word is in 0 to 255
Word has Copy
    finish(self):
        return
replace(previous Word, next Word) Word: return next
read(values [in Word; count], index is in 0 to 1) Word: return values.[index] as Word
Slot has
    finish(self):
        return
    pointer @Word
    write(self mut, value Word):
        self.pointer.[0]:: replace(self.pointer.[0], value)
        return
#entry
main:
    original: 7 as Word
    duplicate: original.copy
    slot: Slot(pointer: @duplicate)
    slot.write(19)
    values [in Word; 2]: [3, 5]
    first is in Word: original
    second is in Word: duplicate
    third is in Word: read(values, 1)
    return when first = 7 and second = 19 and third = 5 then 0 else 1
"#,
    );
    run_in_profiles(&package, input, "run indexing");
}
