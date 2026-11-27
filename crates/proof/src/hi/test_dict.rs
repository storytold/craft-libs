//! Hand-written synthetic Hindi test data. No words, rules or suggestion tables were copied
//! from a system dictionary. This is a small fixture, not a redistributable Hindi dictionary.

pub const AFF: &str = "SET UTF-8
FLAG UTF-8
FORBIDDENWORD F
TRY अआइईउऊऋएऐओऔकखगघङचछजझञटठडढणतथदधनपफबभमयरलवशषसहािीुूृेैोौ्ंःँ
KEY सश|तद
MAP 4
MAP इईिी
MAP उऊुू
MAP सशष
MAP ंँ
REP 1
REP कश क्ष
SFX P Y 2
SFX P ा े ा
SFX P ा ों ा
SFX O Y 1
SFX O 0 ों .
SFX E Y 1
SFX E 0 ें .
PFX X Y 1
PFX X 0 अ .
";

pub const DIC: &str = "14
घर/O
शहर/O
गाँव/O
मित्र/O
किताब/OE
कमरा/P
बच्चा/P
भाषा
पानी
परीक्षण
प्रेम
सूर्य
सफल/X
zqप्रतिबंधित/F
";

pub fn install() {
    crate::hold_dictionary_tests();
    super::install_dictionary(AFF, DIC).unwrap();
}
