//! A tiny synthetic Portuguese Hunspell dictionary for tests (written for these tests; not taken
//! from any real dictionary). Real dictionaries are installed on the system, never bundled.
//!
//! It exercises what VERO-like dictionaries use: `FLAG UTF-8` (multi-byte flag characters —
//! `F` marks a forbidden spelling), continuation classes (`provável` → `prováveis`), the
//! suggestion tables (`TRY`, `MAP` for accents, `REP` for common confusions) and one prefix.

pub const AFF: &str = "SET UTF-8
FLAG UTF-8
FORBIDDENWORD F
TRY aeiousrntmdlcpvgqbzfhjxçãõáéíóúâêôkw
MAP 6
MAP aáàâã
MAP eéê
MAP ií
MAP oóôõ
MAP uú
MAP cç
REP 4
REP seje seja
REP excessão exceção
REP previlégio privilégio
REP analize análise
SFX A Y 2
SFX A a ava a
SFX A a ando a
SFX B N 1
SFX B   0   ável/C   .
SFX C N 1
SFX C   vel   veis   vel
PFX X Y 1
PFX X 0 des .
";

/// Stems (the count line first, as in Hunspell files).
pub const DIC: &str = "76
coração
saudade
saudê
pássaro
saúde
árvore
escrever
computador
praia
conhecer
Brasília
São
Paulo
guarda
roupa
mail
marido
ex
vice
presidente
autoestima
frequência
frequencia/F
exceção
privilégio
análise
seja
trabalho
canta/A
prov/B
cobrir/X
avião
pássaros
amanhã
vocês
coragem
parabéns
está
ele
disse
existem
bem
não
sim
que
com
para
dos
das
pela
pelo
muito
mais
como
porque
aqui
também
já
ser
ter
ir
ver
dar
e
o
é
água
ela
da
do
de
em
no
na
um
uma
";

/// Make [`AFF`]/[`DIC`] the Portuguese dictionary (every test that needs one calls this; they all
/// install the same one, so tests running in parallel agree).
pub fn install() {
    if let Err(e) = super::install_dictionary(AFF, DIC) {
        panic!("test dictionary: {e}");
    }
}
