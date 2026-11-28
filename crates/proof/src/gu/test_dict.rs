//! Hand-written synthetic Gujarati test data. No words, rules or suggestion tables were copied
//! from a system dictionary. This is a small fixture, not a redistributable Gujarati dictionary.

pub const AFF: &str = "SET UTF-8
FLAG UTF-8
FORBIDDENWORD F
TRY અઆઇઈઉઊઋએઐઓઔકખગઘઙચછજઝઞટઠડઢણતથદધનપફબભમયરલવશષસહળક્ષજ્ઞાિીુૂૃૄેૈોૌ્ંઃઁ
KEY ષસ|તર
MAP 4
MAP ઇઈિી
MAP ઉઊુૂ
MAP સશષ
MAP ણન
REP 1
REP કશ ક્ષ
SFX N Y 4
SFX N 0 ો/C .
SFX N 0 ો .
SFX N 0 ઓ/C .
SFX N 0 ઓ .
SFX C Y 4
SFX C 0 ને .
SFX C 0 માં .
SFX C 0 થી .
SFX C 0 નો .
PFX X Y 1
PFX X 0 અ .
";

pub const DIC: &str = "15
ઘર/NX
પુસ્તક/NX
પાણી
શાળા
ભાષા
પરીક્ષણ
પ્રેમ
શહેર
ગામ/NX
મિત્ર/NX
ઝાડ
ફળ
કમળ
સૂર્ય
zqપ્રતિબંધિત/F
";

pub fn install() {
    crate::hold_dictionary_tests();
    super::install_dictionary(AFF, DIC).unwrap();
}
