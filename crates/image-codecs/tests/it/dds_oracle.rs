//! Byte-for-byte oracle for the DDS reader and writer: SHA-256 digests of what the writer makes
//! for uncompressed, BC1, BC2 and BC3 textures, with and without mip chains, from deterministic
//! synthetic images, and of what the reader decodes from those files and from hand-built files
//! in the other stored layouts. The digests were taken from the code before the block encoder and
//! decoder were sped up; any change to them is a change of output.
//!
//! On a mismatch the test prints every digest it computed in the form of [`PINNED`], so a new
//! table can be pasted in once a change of output is intended.

use craft_image_codecs::{ChannelLayout, DdsCompression, DdsOptions, EncodeOptions, Format, Image, decode, encode};
use sha2::{Digest, Sha256};

/// `(case, SHA-256)`, taken from the code before the speed-ups.
const PINNED: &[(&str, &str)] = &[
    ("Rgba-517x389-none-mips0-encode", "e2aee5dcc86cc36e9735cb75e61852324210960092bfb2f2aec6cee9ba54ad16"),
    ("Rgba-517x389-none-mips0-decode", "e0d9e319f260fa29481495313cfa8c3d224a27ad311e7aabd32f7cda8dfb81d2"),
    ("Rgba-517x389-none-mips1-encode", "7611e8174d32962c9643e8f3208b8f30eba2cb85cfe16538c4321e419f3a98ce"),
    ("Rgba-517x389-none-mips1-decode", "e0d9e319f260fa29481495313cfa8c3d224a27ad311e7aabd32f7cda8dfb81d2"),
    ("Rgba-517x389-bc1-mips0-encode", "b6883d729a1eebbdc96176c9264754378d4f7f6bc19d355a12419c0ccfd3ac2a"),
    ("Rgba-517x389-bc1-mips0-decode", "e23704d5f72ba4f2c63d815d5bff3d4c79433fa38122e1e21f1003fa1b12476a"),
    ("Rgba-517x389-bc1-mips1-encode", "554ed42ba6b7c206f779324670629538d8ce12d85ef495ad36954aae2b952917"),
    ("Rgba-517x389-bc1-mips1-decode", "e23704d5f72ba4f2c63d815d5bff3d4c79433fa38122e1e21f1003fa1b12476a"),
    ("Rgba-517x389-bc2-mips0-encode", "473f389b464d2f74b05d44ab2d8c31d4d4cd64648d389e4fe27c27491003821b"),
    ("Rgba-517x389-bc2-mips0-decode", "9d81c15d7f5f0dbc897063726fac6e2f3863818346e1c878326c22ed332ae328"),
    ("Rgba-517x389-bc2-mips1-encode", "1eb8c1a73cb4e6965c60a5ca4322fb66f071cad7439d97d77dbea1a46231724a"),
    ("Rgba-517x389-bc2-mips1-decode", "9d81c15d7f5f0dbc897063726fac6e2f3863818346e1c878326c22ed332ae328"),
    ("Rgba-517x389-bc3-mips0-encode", "0b133915d5ac0376f8b4425cb2dd4a1ace609cf7c1d635be0ff46a37529363e2"),
    ("Rgba-517x389-bc3-mips0-decode", "9c5aeecab34af4c04dd603cf5c379060e6615c07df05ae4e93058bca13a61bc9"),
    ("Rgba-517x389-bc3-mips1-encode", "130e5657e870ee5c03bcfcc2c5e2befbc8a67fc0df1b7e7b71dbb67cc3f0b5e7"),
    ("Rgba-517x389-bc3-mips1-decode", "9c5aeecab34af4c04dd603cf5c379060e6615c07df05ae4e93058bca13a61bc9"),
    ("Rgb-517x389-none-mips0-encode", "6df9ec01a5ae0a8e0fe3b563a7ff670477446682cadd21c24ca5e0cff2b41ed8"),
    ("Rgb-517x389-none-mips0-decode", "7f181edc81e2e38d56c2ed5c81edf480d52caa746e0bef90f0070e171ead89d1"),
    ("Rgb-517x389-none-mips1-encode", "5a48af2cfa7d48455fe49105fc5d0173c6f4fd8187483af923f0af2fbf0af9ef"),
    ("Rgb-517x389-none-mips1-decode", "7f181edc81e2e38d56c2ed5c81edf480d52caa746e0bef90f0070e171ead89d1"),
    ("Rgb-517x389-bc1-mips0-encode", "1e8bf3d0ba485d0892ce771698868bf7029d2c4e63057f327ae3ab39bc57cd4a"),
    ("Rgb-517x389-bc1-mips0-decode", "96feb4a7a3b7416ebc0d5e42271a9d6304131e6cbed25f745d3eef6a40279f8d"),
    ("Rgb-517x389-bc1-mips1-encode", "76f866a8bbb0b3cad335c604cba97d173d01dcbfff4c8930a1d8af7c60924494"),
    ("Rgb-517x389-bc1-mips1-decode", "96feb4a7a3b7416ebc0d5e42271a9d6304131e6cbed25f745d3eef6a40279f8d"),
    ("Rgb-517x389-bc2-mips0-encode", "a8eaab96cf9c6f7926526069ed71f5df8c7f7d43f3ef40a85f1403639c09ac97"),
    ("Rgb-517x389-bc2-mips0-decode", "8e00407a44922477b8d7c05b371f05c18ac3471095990b44aba438bf39db57b2"),
    ("Rgb-517x389-bc2-mips1-encode", "d6fee09087b9a4c9a055051e7aa212a9a0a3b2660e1dfa49697c6fc9754a9aae"),
    ("Rgb-517x389-bc2-mips1-decode", "8e00407a44922477b8d7c05b371f05c18ac3471095990b44aba438bf39db57b2"),
    ("Rgb-517x389-bc3-mips0-encode", "2e177ad42c89f6662ebf2b834b691e10bfaea32a565b740e1bd48c7e4b39cb68"),
    ("Rgb-517x389-bc3-mips0-decode", "8e00407a44922477b8d7c05b371f05c18ac3471095990b44aba438bf39db57b2"),
    ("Rgb-517x389-bc3-mips1-encode", "0b53960adcf0844b47ed5927862901c95fb632af4e85cd94b5ceae59eccd78cd"),
    ("Rgb-517x389-bc3-mips1-decode", "8e00407a44922477b8d7c05b371f05c18ac3471095990b44aba438bf39db57b2"),
    ("GrayA-517x389-none-mips0-encode", "3f8e56201cb76990b3e25b62631666ac113a5340877acd57cdcb1c229127d3f4"),
    ("GrayA-517x389-none-mips0-decode", "43503ba84a7d7579b1ebbe6ba522146f5e63f0116f8dd0eba095364cfba37c5d"),
    ("GrayA-517x389-none-mips1-encode", "6282214743c82e3f74f2347c859ecfd79447cfbc87a2016a238e714f2d150b87"),
    ("GrayA-517x389-none-mips1-decode", "43503ba84a7d7579b1ebbe6ba522146f5e63f0116f8dd0eba095364cfba37c5d"),
    ("GrayA-517x389-bc1-mips0-encode", "32d7065b40986561349f0d76db148b1b62afedb1ce8be6a4e0b209c60c9d861f"),
    ("GrayA-517x389-bc1-mips0-decode", "1cc1cbd355033b5674cc1000b879d4dc8cbb4b53b8dd38d90d8b439550e2be7c"),
    ("GrayA-517x389-bc1-mips1-encode", "4b5e5c9db8ec9847b55e0c51b2e1c1cb41bb277517ef5a7bf59ace7581a7696a"),
    ("GrayA-517x389-bc1-mips1-decode", "1cc1cbd355033b5674cc1000b879d4dc8cbb4b53b8dd38d90d8b439550e2be7c"),
    ("GrayA-517x389-bc2-mips0-encode", "91fd37653f090668328d8feff0407c5998ef2ab96d220bd49e311faf7d768e00"),
    ("GrayA-517x389-bc2-mips0-decode", "ac73339602e5062f78cf76694bde0a4ca538b355170eb424f0bd1cf7d2c12096"),
    ("GrayA-517x389-bc2-mips1-encode", "05d77863968b0086ddf4a1c21ccbc8e1f2d9638ed4dffd114107a1d793ea3311"),
    ("GrayA-517x389-bc2-mips1-decode", "ac73339602e5062f78cf76694bde0a4ca538b355170eb424f0bd1cf7d2c12096"),
    ("GrayA-517x389-bc3-mips0-encode", "bb4057b77e0fa2959d6e79b77ff67090c9dd4d08f3c105b57ebbaea214ffc8c7"),
    ("GrayA-517x389-bc3-mips0-decode", "fdcbd158c32404d09cf1ee38005b2870882e620bcd67c77332d492d07099d665"),
    ("GrayA-517x389-bc3-mips1-encode", "65b2021ea2a385b349d2a6f3a44305e3655f35f3bee39fba5d177bb8c7a5d66a"),
    ("GrayA-517x389-bc3-mips1-decode", "fdcbd158c32404d09cf1ee38005b2870882e620bcd67c77332d492d07099d665"),
    ("Gray-517x389-none-mips0-encode", "fc6cf28e9220780d752ac3be0c9b9bf75412dbea4a54ab603577f78000f235d3"),
    ("Gray-517x389-none-mips0-decode", "13df79103435da32999dff6deac57b02a872aef3fe7dafe580625a554258db30"),
    ("Gray-517x389-none-mips1-encode", "e6f1bb2eb28353a1ead1b72e6e0c78f9490f9e933ee2fe464044ba2804d0a595"),
    ("Gray-517x389-none-mips1-decode", "13df79103435da32999dff6deac57b02a872aef3fe7dafe580625a554258db30"),
    ("Gray-517x389-bc1-mips0-encode", "ba3b1a2567ff1571dfb855b7c5ff8e2da276eb80f5f97db0fae661449bb51822"),
    ("Gray-517x389-bc1-mips0-decode", "7df1ebd46f395e515af2763a0bf7189072a64f20ff811973098ea109df1ed832"),
    ("Gray-517x389-bc1-mips1-encode", "e34896e52b393aa3035e641b43881df1a25ae4aa6261c9bdca34889ec42ea6ac"),
    ("Gray-517x389-bc1-mips1-decode", "7df1ebd46f395e515af2763a0bf7189072a64f20ff811973098ea109df1ed832"),
    ("Gray-517x389-bc2-mips0-encode", "2ebe06ad3ca05dc993a60c58dbae1d36c56516147176b4799648f534e9318b3a"),
    ("Gray-517x389-bc2-mips0-decode", "2adde466f9a4c2c4905d50490bbed4cd5c4de0e3b951d63028702f7ab51d7927"),
    ("Gray-517x389-bc2-mips1-encode", "8730c500b41e9b628cab1c789b2e2a2e4a4ad0350b6c3faaca5e3acd8b423ca7"),
    ("Gray-517x389-bc2-mips1-decode", "2adde466f9a4c2c4905d50490bbed4cd5c4de0e3b951d63028702f7ab51d7927"),
    ("Gray-517x389-bc3-mips0-encode", "e8b62246e29504bbbc92f1c5344846577232e2c12f5c6ab6fae68199bbcf5c3a"),
    ("Gray-517x389-bc3-mips0-decode", "2adde466f9a4c2c4905d50490bbed4cd5c4de0e3b951d63028702f7ab51d7927"),
    ("Gray-517x389-bc3-mips1-encode", "2b1122c5091c8a62c897fa4a133b2a21d19ec14a3ec2c5e0d4855570824e6211"),
    ("Gray-517x389-bc3-mips1-decode", "2adde466f9a4c2c4905d50490bbed4cd5c4de0e3b951d63028702f7ab51d7927"),
    ("Rgba-1x1-none-mips0-encode", "c059322936202ed846ae95243325c1756b3ca2734a310b9b58e957e7f6ecb65c"),
    ("Rgba-1x1-none-mips0-decode", "ac6d8249b868a4df487ad3d4bc2bffa4ec32afed5bb89bcb916dc390872c00a4"),
    ("Rgba-1x1-none-mips1-encode", "c059322936202ed846ae95243325c1756b3ca2734a310b9b58e957e7f6ecb65c"),
    ("Rgba-1x1-none-mips1-decode", "ac6d8249b868a4df487ad3d4bc2bffa4ec32afed5bb89bcb916dc390872c00a4"),
    ("Rgba-1x1-bc1-mips0-encode", "b128a04da98b6d855fd98c6908f9ba47d0acf2e79c1b39d0cf3e5787ee52b23c"),
    ("Rgba-1x1-bc1-mips0-decode", "2c1567f9329e7d7046ebcba26c8b66a872a36d2140c47be6d8ee3fd1bdc7aa26"),
    ("Rgba-1x1-bc1-mips1-encode", "b128a04da98b6d855fd98c6908f9ba47d0acf2e79c1b39d0cf3e5787ee52b23c"),
    ("Rgba-1x1-bc1-mips1-decode", "2c1567f9329e7d7046ebcba26c8b66a872a36d2140c47be6d8ee3fd1bdc7aa26"),
    ("Rgba-1x1-bc2-mips0-encode", "d53707f0da90291d459df619c204c09d0e5537b96055b7c5a7eebb164965a30c"),
    ("Rgba-1x1-bc2-mips0-decode", "e41eb1f8866a8a844c5627ec9e119ea9b5b2c2afb62c2cc40d0b3463e191181f"),
    ("Rgba-1x1-bc2-mips1-encode", "d53707f0da90291d459df619c204c09d0e5537b96055b7c5a7eebb164965a30c"),
    ("Rgba-1x1-bc2-mips1-decode", "e41eb1f8866a8a844c5627ec9e119ea9b5b2c2afb62c2cc40d0b3463e191181f"),
    ("Rgba-1x1-bc3-mips0-encode", "b7a5261aeaa590ee27d74e1ec40688cebfd95a8b32bad510dcf4889659bced99"),
    ("Rgba-1x1-bc3-mips0-decode", "ac6d8249b868a4df487ad3d4bc2bffa4ec32afed5bb89bcb916dc390872c00a4"),
    ("Rgba-1x1-bc3-mips1-encode", "b7a5261aeaa590ee27d74e1ec40688cebfd95a8b32bad510dcf4889659bced99"),
    ("Rgba-1x1-bc3-mips1-decode", "ac6d8249b868a4df487ad3d4bc2bffa4ec32afed5bb89bcb916dc390872c00a4"),
    ("Gray-1x1-none-mips0-encode", "9acce63d02641780f46268c15cb62bdf84b68edc091801d02f62d8840b90e76f"),
    ("Gray-1x1-none-mips0-decode", "5ef973abd913579530066461f86da91115474c93b5a6cf1b73327feeb3650b83"),
    ("Gray-1x1-none-mips1-encode", "9acce63d02641780f46268c15cb62bdf84b68edc091801d02f62d8840b90e76f"),
    ("Gray-1x1-none-mips1-decode", "5ef973abd913579530066461f86da91115474c93b5a6cf1b73327feeb3650b83"),
    ("Gray-1x1-bc1-mips0-encode", "ffec653603d347468446a15fe4101fda9cb34efb48dc69a8d0f403ce2e2fd9d7"),
    ("Gray-1x1-bc1-mips0-decode", "bf04357a38e992a2513e373d00f664b687669a14e1405a2d1b95fb0de6e9a89b"),
    ("Gray-1x1-bc1-mips1-encode", "ffec653603d347468446a15fe4101fda9cb34efb48dc69a8d0f403ce2e2fd9d7"),
    ("Gray-1x1-bc1-mips1-decode", "bf04357a38e992a2513e373d00f664b687669a14e1405a2d1b95fb0de6e9a89b"),
    ("Gray-1x1-bc2-mips0-encode", "3c36aac2c17a7aad99b2c1fa0f4e5b256a9b4421c38406f61961b09c64290178"),
    ("Gray-1x1-bc2-mips0-decode", "bf04357a38e992a2513e373d00f664b687669a14e1405a2d1b95fb0de6e9a89b"),
    ("Gray-1x1-bc2-mips1-encode", "3c36aac2c17a7aad99b2c1fa0f4e5b256a9b4421c38406f61961b09c64290178"),
    ("Gray-1x1-bc2-mips1-decode", "bf04357a38e992a2513e373d00f664b687669a14e1405a2d1b95fb0de6e9a89b"),
    ("Gray-1x1-bc3-mips0-encode", "73704fca801476ab72bf69b80a395de711e9a3fa4f476ce5481ae3858f844922"),
    ("Gray-1x1-bc3-mips0-decode", "bf04357a38e992a2513e373d00f664b687669a14e1405a2d1b95fb0de6e9a89b"),
    ("Gray-1x1-bc3-mips1-encode", "73704fca801476ab72bf69b80a395de711e9a3fa4f476ce5481ae3858f844922"),
    ("Gray-1x1-bc3-mips1-decode", "bf04357a38e992a2513e373d00f664b687669a14e1405a2d1b95fb0de6e9a89b"),
    ("Rgba-2x3-none-mips0-encode", "1f641b8e1cc01924b0d1a06d2d81ad22c759b44a4f640a83ba6339446e13fc6c"),
    ("Rgba-2x3-none-mips0-decode", "af815e7ba5ce6369171335ae94e553524db701f94d152b903f220ee64b9373a6"),
    ("Rgba-2x3-none-mips1-encode", "9be390b308b0961125828ca53894a87101983eafaf54e3c7e3da7b1f992fb518"),
    ("Rgba-2x3-none-mips1-decode", "af815e7ba5ce6369171335ae94e553524db701f94d152b903f220ee64b9373a6"),
    ("Rgba-2x3-bc1-mips0-encode", "743a3d25313924d68732ab399ed92c8e3f693ef26a415a12532918b80db085c4"),
    ("Rgba-2x3-bc1-mips0-decode", "570f9feaf2d322a568b2a87ca652aa0606ff96cde281e24d247d071a9dc4e827"),
    ("Rgba-2x3-bc1-mips1-encode", "711b0088c7c8083760921c3620d9d1adc02d47b5c3b0eb9e674d974fd1d6d641"),
    ("Rgba-2x3-bc1-mips1-decode", "570f9feaf2d322a568b2a87ca652aa0606ff96cde281e24d247d071a9dc4e827"),
    ("Rgba-2x3-bc2-mips0-encode", "b4facaf715dd8208fe02961fb99584b58c219117a99f2b1d15d10a556a9e5251"),
    ("Rgba-2x3-bc2-mips0-decode", "66112c3066ef59936784fa938c26eb0e86772da586442237de5336c48558afbe"),
    ("Rgba-2x3-bc2-mips1-encode", "cad117a2ddd6fe91876efd1dd0b80da0b393f3af21209efc3c1b0f165606a129"),
    ("Rgba-2x3-bc2-mips1-decode", "66112c3066ef59936784fa938c26eb0e86772da586442237de5336c48558afbe"),
    ("Rgba-2x3-bc3-mips0-encode", "b68ced41c6b33037f08b74c492da5688e9d80555c5843f9f323a74d7c86e114e"),
    ("Rgba-2x3-bc3-mips0-decode", "051b60ea47094987d71180aeaa00b3f1b5427429e54812ea28b40f8e945044c8"),
    ("Rgba-2x3-bc3-mips1-encode", "88e0fd6d04c896880d03731d6df1528f22290c1738b303eb2b09fe84d7882a64"),
    ("Rgba-2x3-bc3-mips1-decode", "051b60ea47094987d71180aeaa00b3f1b5427429e54812ea28b40f8e945044c8"),
    ("Gray-2x3-none-mips0-encode", "7e9d26894ca9a800b93f5dae0e5b58b18a6cbcaea56eacc657aba40a1933c476"),
    ("Gray-2x3-none-mips0-decode", "d21c9cddccb970575eba36093410245f0a0fb15049fc96311874191f868de0d5"),
    ("Gray-2x3-none-mips1-encode", "d2ee41ea49462e1d7be568cfcbf867c6395646710ba321c2dac9db319294bb5e"),
    ("Gray-2x3-none-mips1-decode", "d21c9cddccb970575eba36093410245f0a0fb15049fc96311874191f868de0d5"),
    ("Gray-2x3-bc1-mips0-encode", "000d5d9121042f7b88411d8e08a548a2c804b8b4b73b40bf317cff61899a5445"),
    ("Gray-2x3-bc1-mips0-decode", "1a8f46dd535383398fbeebe5fdfca9bb256fa3b3a7b8552af34b67b09865e89d"),
    ("Gray-2x3-bc1-mips1-encode", "07346ec09eb8d9b489e7dd50bd73851342205594ac985544bfe58b6bda73b21b"),
    ("Gray-2x3-bc1-mips1-decode", "1a8f46dd535383398fbeebe5fdfca9bb256fa3b3a7b8552af34b67b09865e89d"),
    ("Gray-2x3-bc2-mips0-encode", "5db43dcd4adec962a7a12c6f8da8bf05d3dc2e3d608ca3fa341e36b43c218c3d"),
    ("Gray-2x3-bc2-mips0-decode", "afabd75898a95baac868e9e14905c2d89e90767750217e4c9ac84b1caeef41bc"),
    ("Gray-2x3-bc2-mips1-encode", "3effae05c47854dfb5f20ed12e2c1588236464f17fbf5dd13e61958e5097a796"),
    ("Gray-2x3-bc2-mips1-decode", "afabd75898a95baac868e9e14905c2d89e90767750217e4c9ac84b1caeef41bc"),
    ("Gray-2x3-bc3-mips0-encode", "11c48dce5b7545cf3eafd1f01fc6b977b13cb7c7e8f2ef7ea1406a8f04b86cf9"),
    ("Gray-2x3-bc3-mips0-decode", "afabd75898a95baac868e9e14905c2d89e90767750217e4c9ac84b1caeef41bc"),
    ("Gray-2x3-bc3-mips1-encode", "6b7540afc583554e70f4131b4a0d8648eed65100293ab9b7060afcbd33751e64"),
    ("Gray-2x3-bc3-mips1-decode", "afabd75898a95baac868e9e14905c2d89e90767750217e4c9ac84b1caeef41bc"),
    ("Rgba-3x2-none-mips0-encode", "e821c1620e4e53588908268da45702924911ed756f3257cb0a21727748b736a6"),
    ("Rgba-3x2-none-mips0-decode", "d4583449852070512fa3cb16c5132834ccee2f9dd3a6c0d6c8fa5bbbc2935e8d"),
    ("Rgba-3x2-none-mips1-encode", "f79a699612ef8d8347a5e83af2fd9ee915eda35388bc86b4243b265f988fc3d8"),
    ("Rgba-3x2-none-mips1-decode", "d4583449852070512fa3cb16c5132834ccee2f9dd3a6c0d6c8fa5bbbc2935e8d"),
    ("Rgba-3x2-bc1-mips0-encode", "1c106b532f6afc132538b2bb2c1eb9e6bb075a448d19b6af2c67b4c31415e3b2"),
    ("Rgba-3x2-bc1-mips0-decode", "f0e63ce55598b6c58c601a5cf3415522113181a08bc28f4d8cd499d160352cdd"),
    ("Rgba-3x2-bc1-mips1-encode", "23d86eba138f7c4492f9955dd0a29eb1a675d69f1e612b54e783837d9414cb14"),
    ("Rgba-3x2-bc1-mips1-decode", "f0e63ce55598b6c58c601a5cf3415522113181a08bc28f4d8cd499d160352cdd"),
    ("Rgba-3x2-bc2-mips0-encode", "74a68d7b1cc0173f1e8a6e0f6101ffa010757df0434fcf5636db808ca20b341e"),
    ("Rgba-3x2-bc2-mips0-decode", "bc63bba15c1bbe19cdef66535870c7dfeeae26a4686557e1530ae64a2b2253a5"),
    ("Rgba-3x2-bc2-mips1-encode", "d28d7a4a7ee17d0f58f27ebef8d682a7fed42778a60e09186af523198a2f3562"),
    ("Rgba-3x2-bc2-mips1-decode", "bc63bba15c1bbe19cdef66535870c7dfeeae26a4686557e1530ae64a2b2253a5"),
    ("Rgba-3x2-bc3-mips0-encode", "fa0a02ba9822a007f1654d618b5225332f3934e5f07c51f8ca474e26658e2c45"),
    ("Rgba-3x2-bc3-mips0-decode", "ee0221751e1d96bf76552df65a7e54d7711f834cf0431fd17a3a6542473b4ceb"),
    ("Rgba-3x2-bc3-mips1-encode", "3a3c51f596a82ae666c6157da4266c3330e8e2e0744947e0ce663deb66e67ad6"),
    ("Rgba-3x2-bc3-mips1-decode", "ee0221751e1d96bf76552df65a7e54d7711f834cf0431fd17a3a6542473b4ceb"),
    ("Gray-3x2-none-mips0-encode", "6e1e49ee238e40108f1e934946b0d2498eb97f6824a82d4d71011759a58b83a4"),
    ("Gray-3x2-none-mips0-decode", "e333fde639747f66aa4d74dbeae648566d6735ecc47de220e07b9da351e1bf37"),
    ("Gray-3x2-none-mips1-encode", "906e16afa18db0290bec758a1a124de7a99255871344d462f797db54dbd30b4b"),
    ("Gray-3x2-none-mips1-decode", "e333fde639747f66aa4d74dbeae648566d6735ecc47de220e07b9da351e1bf37"),
    ("Gray-3x2-bc1-mips0-encode", "df204b170cd24f0375772d65b4f0548a824a4602c453cf891c9cc4aafc4b32ad"),
    ("Gray-3x2-bc1-mips0-decode", "869a8202b3721a8d2bf232e75e9a91a271cf2ebad85c8df6a5260cb9c9b6b1ab"),
    ("Gray-3x2-bc1-mips1-encode", "d06f38b5c40156888fe2ea04343f46d4753a80f83f63d80b3fd7ce82c5d7fe94"),
    ("Gray-3x2-bc1-mips1-decode", "869a8202b3721a8d2bf232e75e9a91a271cf2ebad85c8df6a5260cb9c9b6b1ab"),
    ("Gray-3x2-bc2-mips0-encode", "1832e5006137ae1583d34ba1bfab9c7b85f55791996f7cdf21ed799f3939c4f4"),
    ("Gray-3x2-bc2-mips0-decode", "869a8202b3721a8d2bf232e75e9a91a271cf2ebad85c8df6a5260cb9c9b6b1ab"),
    ("Gray-3x2-bc2-mips1-encode", "41d49c9cce6cbbdf1676033a9233ed774f339d92e8159d1d1b0cbc8503cfe72b"),
    ("Gray-3x2-bc2-mips1-decode", "869a8202b3721a8d2bf232e75e9a91a271cf2ebad85c8df6a5260cb9c9b6b1ab"),
    ("Gray-3x2-bc3-mips0-encode", "377e3d02919641d634bbe02cadb513237fcd90f540b8769c5e2a214d5ccf05e4"),
    ("Gray-3x2-bc3-mips0-decode", "869a8202b3721a8d2bf232e75e9a91a271cf2ebad85c8df6a5260cb9c9b6b1ab"),
    ("Gray-3x2-bc3-mips1-encode", "70b03c1901946a24abf34c60dd7bd9cc124df43a251a82622c15d56b0e2b2588"),
    ("Gray-3x2-bc3-mips1-decode", "869a8202b3721a8d2bf232e75e9a91a271cf2ebad85c8df6a5260cb9c9b6b1ab"),
    ("Rgba-5x5-none-mips0-encode", "be7677f4c86d5f3c0ba75ec04d36bcfd708bca44c9f6720c4b7494c290014491"),
    ("Rgba-5x5-none-mips0-decode", "00dc5f1020a7de3a3bc5ee21e7a767d09dab9c9831e840290277af90d1e1e3be"),
    ("Rgba-5x5-none-mips1-encode", "284c04f628c256ba546815e46df2b837ee997d90834b8a0f095f20e17ba36102"),
    ("Rgba-5x5-none-mips1-decode", "00dc5f1020a7de3a3bc5ee21e7a767d09dab9c9831e840290277af90d1e1e3be"),
    ("Rgba-5x5-bc1-mips0-encode", "a5350786ccbd004a88ca541c0d4d7cdf313f71fec9485a45acd380f318846435"),
    ("Rgba-5x5-bc1-mips0-decode", "26559d0fc43534fad7318b66672ef9de924b6408b2528c00cb6be76a56a94654"),
    ("Rgba-5x5-bc1-mips1-encode", "67e06000b2ee88a6e8e749b20ee6ec0016d294f2fba57e9773aea3a7addd1883"),
    ("Rgba-5x5-bc1-mips1-decode", "26559d0fc43534fad7318b66672ef9de924b6408b2528c00cb6be76a56a94654"),
    ("Rgba-5x5-bc2-mips0-encode", "6cf909177531b9ed88be4ffd031055a7b1cbcfab0c6082ca33913720d631f834"),
    ("Rgba-5x5-bc2-mips0-decode", "060ad06944e973f57c866137f9bd8c903a4edd30af275f9ee92fadff54a0a4bb"),
    ("Rgba-5x5-bc2-mips1-encode", "0590d1aa5b074abb89262432a3b6b5c3ff12edca0d454069dadfe20be690e727"),
    ("Rgba-5x5-bc2-mips1-decode", "060ad06944e973f57c866137f9bd8c903a4edd30af275f9ee92fadff54a0a4bb"),
    ("Rgba-5x5-bc3-mips0-encode", "8d24e90ef90f819353e1bbbb1ab1163463dbdf8d9aaeb89b3acbc8cf2d3dd625"),
    ("Rgba-5x5-bc3-mips0-decode", "d3238486581875abcd37dfbe03e8fbea7aa4fcadb1fef866899deafb67ac38f9"),
    ("Rgba-5x5-bc3-mips1-encode", "2c331cbc46a45536bc1ae50cff9a692bae969748aebfa13e93c0269055ad5031"),
    ("Rgba-5x5-bc3-mips1-decode", "d3238486581875abcd37dfbe03e8fbea7aa4fcadb1fef866899deafb67ac38f9"),
    ("Gray-5x5-none-mips0-encode", "8527392cacb17a19b3e1c0f7cbe05ea4abd7fba93a7952d30967047a2e317f3b"),
    ("Gray-5x5-none-mips0-decode", "70193f4748cd97b7676693076afe2f1c20afa0dc37b4eb89ff3752114a0e0321"),
    ("Gray-5x5-none-mips1-encode", "6c5bd6095bbed4d39c15a6b50dcb5da8d5da48aabf696d2fbfcaf9bd1954c69d"),
    ("Gray-5x5-none-mips1-decode", "70193f4748cd97b7676693076afe2f1c20afa0dc37b4eb89ff3752114a0e0321"),
    ("Gray-5x5-bc1-mips0-encode", "fc7740ff5a801fdcc82e645d25240b44899a64117e70d739c8cba1c17c5a0841"),
    ("Gray-5x5-bc1-mips0-decode", "65a82c1c1048d612c3b5e947603206571f088d1348e6acc42b9e2c014121e227"),
    ("Gray-5x5-bc1-mips1-encode", "3953cd92035a27902d5feaf61ba11b4696c8593fb139fbdb6fda0167ac6d7a58"),
    ("Gray-5x5-bc1-mips1-decode", "65a82c1c1048d612c3b5e947603206571f088d1348e6acc42b9e2c014121e227"),
    ("Gray-5x5-bc2-mips0-encode", "4b2c26f66303f83c35c4f0ca8e306afbde35cfac5e9986cb6dc2ab33696a21c1"),
    ("Gray-5x5-bc2-mips0-decode", "65a82c1c1048d612c3b5e947603206571f088d1348e6acc42b9e2c014121e227"),
    ("Gray-5x5-bc2-mips1-encode", "047e4d8df50b60f8936e8da938b7c62217da128aaf88a25a9384aa69a3701ce9"),
    ("Gray-5x5-bc2-mips1-decode", "65a82c1c1048d612c3b5e947603206571f088d1348e6acc42b9e2c014121e227"),
    ("Gray-5x5-bc3-mips0-encode", "9c31c80f1e254a0e590927faf77fe88007d01c93d7a99280b7d632c4d680a8c4"),
    ("Gray-5x5-bc3-mips0-decode", "65a82c1c1048d612c3b5e947603206571f088d1348e6acc42b9e2c014121e227"),
    ("Gray-5x5-bc3-mips1-encode", "e046981ea9208a3da3cabbb6dd3f3f439db09d8713e6594e099bd7a3ebcd50dc"),
    ("Gray-5x5-bc3-mips1-decode", "65a82c1c1048d612c3b5e947603206571f088d1348e6acc42b9e2c014121e227"),
    ("Rgba-1x9-none-mips0-encode", "94676df714a0e07fd91981b3f9795da76df174319d551573bf62a7324ceae4fb"),
    ("Rgba-1x9-none-mips0-decode", "dbcf64981f64ab690faa081bf7a90405bc9710ca5789f2bbf63d643469d1dd78"),
    ("Rgba-1x9-none-mips1-encode", "2614a6ecc2aa845cab81819cd8bcd8b84624d693df6979cba3df91e100f27361"),
    ("Rgba-1x9-none-mips1-decode", "dbcf64981f64ab690faa081bf7a90405bc9710ca5789f2bbf63d643469d1dd78"),
    ("Rgba-1x9-bc1-mips0-encode", "b7f1466fcebfb73bd6e3b6288d3a8adde258fd9801c694fd44d0f92fbde26c61"),
    ("Rgba-1x9-bc1-mips0-decode", "7d3471a56c9ca5332bccb8943e9d5a01105e0ddfd432b403e77841137f927787"),
    ("Rgba-1x9-bc1-mips1-encode", "4c7c22ba8e2653f31cd9114857d8ba63db6ac180507e40e8c567b61b9c6781d9"),
    ("Rgba-1x9-bc1-mips1-decode", "7d3471a56c9ca5332bccb8943e9d5a01105e0ddfd432b403e77841137f927787"),
    ("Rgba-1x9-bc2-mips0-encode", "9dc6e68ae27d0709f8859d6efa0dfc4028d9fbba2cc0aef106973c998417f5d1"),
    ("Rgba-1x9-bc2-mips0-decode", "c9dd7bf6d44932b599eeb73b54c6f778880db038768110b005f1886f6993e1db"),
    ("Rgba-1x9-bc2-mips1-encode", "5ca6d64da612af3b9018cefa50e98cb9097c2eb3dbda8f862571057ba02d4312"),
    ("Rgba-1x9-bc2-mips1-decode", "c9dd7bf6d44932b599eeb73b54c6f778880db038768110b005f1886f6993e1db"),
    ("Rgba-1x9-bc3-mips0-encode", "f1d5194773bea0b427702900c928d6829f35cbfd32ec0e76b3ee882670597a81"),
    ("Rgba-1x9-bc3-mips0-decode", "feda0633d1e41776c56257893bbe2be5317f57bb771152e1a4336558c16c013c"),
    ("Rgba-1x9-bc3-mips1-encode", "4faa40254411822689f7d6b356c447e58f89a00ea05de3f7c43253164d83823c"),
    ("Rgba-1x9-bc3-mips1-decode", "feda0633d1e41776c56257893bbe2be5317f57bb771152e1a4336558c16c013c"),
    ("Gray-1x9-none-mips0-encode", "deb41e75756b10819d44b9840171a088a5d0169f56962a055939131986b7ebd1"),
    ("Gray-1x9-none-mips0-decode", "2713191d6e180f74fa7825564575abcf7cc40c34f09c2cb8ccd4d2b02a675bb0"),
    ("Gray-1x9-none-mips1-encode", "2d0f36be14a693e7c0380d70841d56cc857e4b81d1a4acbd94eeb18fca0c9216"),
    ("Gray-1x9-none-mips1-decode", "2713191d6e180f74fa7825564575abcf7cc40c34f09c2cb8ccd4d2b02a675bb0"),
    ("Gray-1x9-bc1-mips0-encode", "0723a0ecd94984d4a24890abc427611135b5add4c8ced0cd570867abe49217df"),
    ("Gray-1x9-bc1-mips0-decode", "dcac5ac2d759bc4166ec0d061863f6bf9a34a03a382ac1d88ebdec3e750ecf3e"),
    ("Gray-1x9-bc1-mips1-encode", "b1c38854773fde2bf2fc6e0c13ea5d8f6c92a19c1cb7af5c2986851522f2c5c2"),
    ("Gray-1x9-bc1-mips1-decode", "dcac5ac2d759bc4166ec0d061863f6bf9a34a03a382ac1d88ebdec3e750ecf3e"),
    ("Gray-1x9-bc2-mips0-encode", "b55d775f6f6029fa6d58ea4bea1abeaed349478665874ae2d5a53be14389c595"),
    ("Gray-1x9-bc2-mips0-decode", "dcac5ac2d759bc4166ec0d061863f6bf9a34a03a382ac1d88ebdec3e750ecf3e"),
    ("Gray-1x9-bc2-mips1-encode", "2e90ad054e173a392042126fd8674516fd8dc928c098238f92b7d434181a5895"),
    ("Gray-1x9-bc2-mips1-decode", "dcac5ac2d759bc4166ec0d061863f6bf9a34a03a382ac1d88ebdec3e750ecf3e"),
    ("Gray-1x9-bc3-mips0-encode", "3316e2f5db9bccd337b9dfede5d8c7a4555a645939682f4fc323e746722ef850"),
    ("Gray-1x9-bc3-mips0-decode", "dcac5ac2d759bc4166ec0d061863f6bf9a34a03a382ac1d88ebdec3e750ecf3e"),
    ("Gray-1x9-bc3-mips1-encode", "0ce9b894539c7e6d22531884a6e352aef26fc899ccf97d8a86878f6f5a68d969"),
    ("Gray-1x9-bc3-mips1-decode", "dcac5ac2d759bc4166ec0d061863f6bf9a34a03a382ac1d88ebdec3e750ecf3e"),
    ("Rgba-9x1-none-mips0-encode", "29bc4ef0ce9120982b331bf34a460a75f7c086b630ea6710ddd907b01125cc30"),
    ("Rgba-9x1-none-mips0-decode", "666aa0014607ab13b7141c423c1fb137bda8a5c92973b3252c0556247383d80b"),
    ("Rgba-9x1-none-mips1-encode", "455e7e2173deb612b28ac2ea2cacaf66c740150b3702e2d027bf51dc1123217f"),
    ("Rgba-9x1-none-mips1-decode", "666aa0014607ab13b7141c423c1fb137bda8a5c92973b3252c0556247383d80b"),
    ("Rgba-9x1-bc1-mips0-encode", "8e21b90879f2768fe0341375245b42a1645bc776ef88f81d700cd7e4f538ece3"),
    ("Rgba-9x1-bc1-mips0-decode", "f6b1a1af12998d9db067870ff988d9b527df09ebfbe6a06710474247155655a8"),
    ("Rgba-9x1-bc1-mips1-encode", "034c32c5a3025a72161e02ea5422ead0253490df2abf37030a048c93a301b386"),
    ("Rgba-9x1-bc1-mips1-decode", "f6b1a1af12998d9db067870ff988d9b527df09ebfbe6a06710474247155655a8"),
    ("Rgba-9x1-bc2-mips0-encode", "7d9fbf241277124a62bd0bc466e687d915435cbc083cf0f228694546b612f75e"),
    ("Rgba-9x1-bc2-mips0-decode", "5360be17f525cac699135d57837231fbeb993a906c0f7efbffea6c18da4bb9b4"),
    ("Rgba-9x1-bc2-mips1-encode", "c7fb6cb488b40b0d08f9958d8d19e3e3f8ac19ce96cd67f99b4bf6d65fb7b0fa"),
    ("Rgba-9x1-bc2-mips1-decode", "5360be17f525cac699135d57837231fbeb993a906c0f7efbffea6c18da4bb9b4"),
    ("Rgba-9x1-bc3-mips0-encode", "1e9ad28a5057cd576ad5f47625ccbdd836e683fd321428ac30760a15753a3b0e"),
    ("Rgba-9x1-bc3-mips0-decode", "a367950e128431e9c0728de32358905486a9f53efef56827cf8f7fa71c9382af"),
    ("Rgba-9x1-bc3-mips1-encode", "0f36472c2a24a64e07893879b60a1fc15e492038a669a4fdf0f586942ff7d197"),
    ("Rgba-9x1-bc3-mips1-decode", "a367950e128431e9c0728de32358905486a9f53efef56827cf8f7fa71c9382af"),
    ("Gray-9x1-none-mips0-encode", "4d65e2aad9e331cfc4edf2c0aaa58673c7ba90f1ad37cfa9c6a31095976ebc0a"),
    ("Gray-9x1-none-mips0-decode", "105c3ae0c54915506d98a823112f37182f15e2cbed571b5161c4dfa1007575b5"),
    ("Gray-9x1-none-mips1-encode", "0943b2616b8800af4415b53893ea301f1020e062b20f37095f1be9653a31e9bd"),
    ("Gray-9x1-none-mips1-decode", "105c3ae0c54915506d98a823112f37182f15e2cbed571b5161c4dfa1007575b5"),
    ("Gray-9x1-bc1-mips0-encode", "2c41ef37fc366203a20e1970feecdc8b619659215412a916221ccaa2b273430e"),
    ("Gray-9x1-bc1-mips0-decode", "afd18b30efff0e3eb4b59cf01dcbbb06691c3af6028596f8de9efdf370ea915a"),
    ("Gray-9x1-bc1-mips1-encode", "d2de0189a1e7a6a70f9a5d4ea7c7a53e064b7d1511f9b183a8dc31268ddb6ac6"),
    ("Gray-9x1-bc1-mips1-decode", "afd18b30efff0e3eb4b59cf01dcbbb06691c3af6028596f8de9efdf370ea915a"),
    ("Gray-9x1-bc2-mips0-encode", "766180145e22838dc950a136791fb350d51821e39c58270be394357228462fc1"),
    ("Gray-9x1-bc2-mips0-decode", "afd18b30efff0e3eb4b59cf01dcbbb06691c3af6028596f8de9efdf370ea915a"),
    ("Gray-9x1-bc2-mips1-encode", "476abc493a613766ad73fdc6a89ab765a68677ef6623370472dfea2e3d531847"),
    ("Gray-9x1-bc2-mips1-decode", "afd18b30efff0e3eb4b59cf01dcbbb06691c3af6028596f8de9efdf370ea915a"),
    ("Gray-9x1-bc3-mips0-encode", "b2f6f7f90bd0530857b272a6b39ac60e20ff2c65f5c0f71de543276d48fc5a30"),
    ("Gray-9x1-bc3-mips0-decode", "afd18b30efff0e3eb4b59cf01dcbbb06691c3af6028596f8de9efdf370ea915a"),
    ("Gray-9x1-bc3-mips1-encode", "4376a76e8467759cd485d2241c6e1553b6d49e6eb2f83a466d71ca68754776cd"),
    ("Gray-9x1-bc3-mips1-decode", "afd18b30efff0e3eb4b59cf01dcbbb06691c3af6028596f8de9efdf370ea915a"),
    ("Rgba-6x13-none-mips0-encode", "f603fdcab8e987e1f4b023ec68a14d8c03808952fe429338669977f33131c5a8"),
    ("Rgba-6x13-none-mips0-decode", "35c18e04c37154e00ba7a544476335c4e234d8390803eb8f370351551652e49c"),
    ("Rgba-6x13-none-mips1-encode", "5254f124be9189174a400affb9192419d6350f6f3361d24cf417ea42c8d33411"),
    ("Rgba-6x13-none-mips1-decode", "35c18e04c37154e00ba7a544476335c4e234d8390803eb8f370351551652e49c"),
    ("Rgba-6x13-bc1-mips0-encode", "f1b5e39e9a7308be64aa3531cfd487116a4e7f3ea309b694292c9738721ac872"),
    ("Rgba-6x13-bc1-mips0-decode", "0a63216b434cd9b24807ea58c6d63888d68e84fa4834a34a8068f7fbf054449d"),
    ("Rgba-6x13-bc1-mips1-encode", "13e5b50aa32a32d3dc71d75426c773bb36b191bd97c16c6f2ee0d76f4ee4f54b"),
    ("Rgba-6x13-bc1-mips1-decode", "0a63216b434cd9b24807ea58c6d63888d68e84fa4834a34a8068f7fbf054449d"),
    ("Rgba-6x13-bc2-mips0-encode", "0e0f5815d03c6ec3b8d58715beacdbe67a3dde10bf3f3b635425dc93073dbb4d"),
    ("Rgba-6x13-bc2-mips0-decode", "14d744c8aff8c596f409a6a5c7599bca6479e8e084aa63d99660a44bfe55be6e"),
    ("Rgba-6x13-bc2-mips1-encode", "4d007cb53697f394feb540a4745818c1f8a9df4a47819eac38b2102039e9421c"),
    ("Rgba-6x13-bc2-mips1-decode", "14d744c8aff8c596f409a6a5c7599bca6479e8e084aa63d99660a44bfe55be6e"),
    ("Rgba-6x13-bc3-mips0-encode", "db7fc9607849f169b3e1a7a8e100a7515bdce53ff4f1db7a5fbb6cfc1d1b8162"),
    ("Rgba-6x13-bc3-mips0-decode", "33ba59f2acc8ab6877b8474a4b2cf063a0ae54b4a27afc64fde51313ba4ca6bc"),
    ("Rgba-6x13-bc3-mips1-encode", "c985feb0187e7ac21aa829e25c5e5fe08970c58e10aaf71b96416a7969c55e68"),
    ("Rgba-6x13-bc3-mips1-decode", "33ba59f2acc8ab6877b8474a4b2cf063a0ae54b4a27afc64fde51313ba4ca6bc"),
    ("Gray-6x13-none-mips0-encode", "f63bb759c82a74584022a92fa9dfb64f36f3709446e279157942bafef6edc9b9"),
    ("Gray-6x13-none-mips0-decode", "71201dd954c691aff18dab0a886cb7f10c15dbb33fca26dcc9f1047d28382190"),
    ("Gray-6x13-none-mips1-encode", "435bedffe3efccd86a63533671dd55c7cdc982ea176383adb7e56c4745d38d28"),
    ("Gray-6x13-none-mips1-decode", "71201dd954c691aff18dab0a886cb7f10c15dbb33fca26dcc9f1047d28382190"),
    ("Gray-6x13-bc1-mips0-encode", "f8cb01936384bbff861cbebfd7149753b31f185b52de437e03074102b1056abf"),
    ("Gray-6x13-bc1-mips0-decode", "48669b49d5aec2ab69c120c938838ba206d4e2d589399d2bccf0df5580b48194"),
    ("Gray-6x13-bc1-mips1-encode", "f09c7c8bc3def200c72ed32545de294bbcb5436dbc46e249e5d5d2b8ac987a64"),
    ("Gray-6x13-bc1-mips1-decode", "48669b49d5aec2ab69c120c938838ba206d4e2d589399d2bccf0df5580b48194"),
    ("Gray-6x13-bc2-mips0-encode", "a2b00acf35735c638e07c7468dd8d5ab3405a8d69c653cf95a369538168b8e85"),
    ("Gray-6x13-bc2-mips0-decode", "48669b49d5aec2ab69c120c938838ba206d4e2d589399d2bccf0df5580b48194"),
    ("Gray-6x13-bc2-mips1-encode", "40769615b4e3117ade50c33e69a51a3f2f1601018c49ea216971955bed74b2ea"),
    ("Gray-6x13-bc2-mips1-decode", "48669b49d5aec2ab69c120c938838ba206d4e2d589399d2bccf0df5580b48194"),
    ("Gray-6x13-bc3-mips0-encode", "a429bc2f9725fb23f84ac6bff8bd6a1faaf0ab2808912515069b90c002bbf460"),
    ("Gray-6x13-bc3-mips0-decode", "48669b49d5aec2ab69c120c938838ba206d4e2d589399d2bccf0df5580b48194"),
    ("Gray-6x13-bc3-mips1-encode", "df1d533175cc807a8474701fe4cbdecd694fadd03082d506be50d8716e3e138f"),
    ("Gray-6x13-bc3-mips1-decode", "48669b49d5aec2ab69c120c938838ba206d4e2d589399d2bccf0df5580b48194"),
    ("dx10-rgba8-decode", "154284a9d25eef84bb2ee0b8b1c2d3cd95149dbae9a61cbe0ddb0f4e59cd1482"),
    ("dx10-bgra8-decode", "ec571c9b54af06a0e63fed4b8b00a878d634f62297e72a22372c38fec561b08f"),
    ("dx10-bgrx8-decode", "1e9e7678fcd37dcb5c61047cadac102a2053c9d738fab17c55d9d4af573aea99"),
    ("legacy-rgba8-decode", "387f6d2b46545aa0188614c6c407a4c7498f065f3124964b92992a304c4e997d"),
    ("legacy-bgra8-padded-decode", "5d4a59bd952afd03eb206aa2e3eaeb652d1d6abd1bf2ada8d8bc67e153fd0b3b"),
    ("legacy-rgb24-decode", "0d33d20540b8d8714fb2f50193505188a60a743edbca6729806210a990bcebb7"),
    ("legacy-rgb565-decode", "e0529a28e87837cea810bee99263a4d3c1ec99bfa6547bff583fc5848b7319e9"),
    ("legacy-argb4444-decode", "0cf215d8c21faeff39c8b4e37acb8a72c69a873b7d132746c66c2f4a6277d58a"),
    ("legacy-l8-padded-decode", "0c72254df5d58d8a1b4c62d1da03548c10a058dab179d7486a1e1c394a4b1cee"),
    ("legacy-l8a8-decode", "066ff8e0a187094a1feb4bdc57e1cb8a99bc6ed3d69647c87c9d54a093b6815a"),
    ("legacy-dxt1-noise-decode", "78d4ec7cec69249f8b065667eb66a23b23e82a6b7ab167043539976ae874c433"),
    ("legacy-dxt3-noise-decode", "2dae7a8abe345ca37af54ad56e65dcb8a6fc1837f233cc81b5693b0b547babc8"),
    ("legacy-dxt5-noise-decode", "5a6edeb62a5d6043a474de25da0611a550cf81716a8960dbf57feaebddf7d950"),
    ("dx10-bc1-noise-decode", "5c6ea27a01ccb2a1ab3badea75551a9d3209a0176335871186362cc7e1923ead"),
    ("dx10-bc3-noise-decode", "9c5376a1d714471d575344920380755e40f5be6cd4cb54add5b982d833276823"),
];

/// The compressions the digests cover, listed here rather than taken from `DdsCompression::ALL`
/// so a compression added later doesn't change the set.
const COMPRESSIONS: [DdsCompression; 4] = [DdsCompression::None, DdsCompression::Bc1, DdsCompression::Bc2, DdsCompression::Bc3];

/// A 64-bit LCG (Knuth's MMIX constants) with a fixed seed: the same noise on every run.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u8 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as u8
    }
}

/// An RGBA test picture in four vertical bands: smooth ramps with mid alpha, noise in every
/// channel, flat 8×8 tiles of a few colours (whole blocks of one colour, opaque), and a band
/// where alpha jumps between 0, 255 and in-between values over gentle colour.
fn picture(w: u32, h: u32, seed: u64) -> Vec<u8> {
    let mut rng = Lcg(seed);
    let tiles: [[u8; 4]; 5] = [[12, 200, 77, 255], [255, 255, 255, 255], [0, 0, 0, 255], [255, 0, 255, 255], [90, 91, 92, 255]];
    let band = w.div_ceil(4).max(1);
    let mut data = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h {
        for x in 0..w {
            let noise = [rng.next(), rng.next(), rng.next(), rng.next()];
            let px = match x / band {
                0 => [(x * 2) as u8, (y * 3) as u8, (x + y) as u8, (64 + y % 128) as u8],
                1 => noise,
                2 => tiles[((x / 8 + y / 8) % 5) as usize],
                _ => {
                    let alpha = match (x / 3 + y / 5) % 4 {
                        0 => 0,
                        1 => 255,
                        2 => 128,
                        _ => noise[3],
                    };
                    [(y % 256) as u8, 128, (255 - y % 256) as u8, alpha]
                }
            };
            data.extend(px);
        }
    }
    data
}

/// The picture in `layout`, its channels picked from the RGBA picture without any conversion
/// formula: gray is red, and gray's alpha is alpha.
fn image(w: u32, h: u32, layout: ChannelLayout, seed: u64) -> Image {
    let rgba = picture(w, h, seed);
    let data = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| match layout {
            ChannelLayout::Gray => vec![p[0]],
            ChannelLayout::GrayA => vec![p[0], p[3]],
            ChannelLayout::Rgb => vec![p[0], p[1], p[2]],
            _ => p.to_vec(),
        })
        .collect();
    Image::from_u8(w, h, layout, data).unwrap()
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// A decoded image's digest: its size and layout, then its samples.
fn image_digest(img: &Image) -> String {
    let mut h = Sha256::new();
    h.update(img.width().to_le_bytes());
    h.update(img.height().to_le_bytes());
    h.update(format!("{:?}", img.layout()).as_bytes());
    h.update(img.data());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

const DDPF_ALPHAPIXELS: u32 = 0x1;
const DDPF_FOURCC: u32 = 0x4;
const DDPF_RGB: u32 = 0x40;
const DDPF_LUMINANCE: u32 = 0x2_0000;
const DDSD_PITCH: u32 = 0x8;

/// A DDS_HEADER for a single-level 2D texture, with `pitch` set (and flagged) when given.
fn header(width: u32, height: u32, pf_flags: u32, fourcc: &[u8; 4], bits: u32, masks: [u32; 4], pitch: Option<u32>) -> Vec<u8> {
    let mut b = vec![0u8; 128];
    b[0..4].copy_from_slice(b"DDS ");
    let mut put = |at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    put(4, 124);
    put(8, 0x1007 | if pitch.is_some() { DDSD_PITCH } else { 0 });
    put(12, height);
    put(16, width);
    put(20, pitch.unwrap_or(0));
    put(76, 32);
    put(80, pf_flags);
    put(88, bits);
    for (i, m) in masks.into_iter().enumerate() {
        put(92 + 4 * i, m);
    }
    put(108, 0x1000);
    b[84..88].copy_from_slice(fourcc);
    b
}

/// A DX10 texture header for DXGI format `dxgi`.
fn dx10(width: u32, height: u32, dxgi: u32) -> Vec<u8> {
    let mut b = header(width, height, DDPF_FOURCC, b"DX10", 0, [0; 4], None);
    for v in [dxgi, 3, 0, 1, 0] {
        b.extend(v.to_le_bytes());
    }
    b
}

/// `len` bytes of noise.
fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut rng = Lcg(seed);
    (0..len).map(|_| rng.next()).collect()
}

/// Hand-built files in the layouts the writer never makes: RGBA and BGRA through the DX10
/// header, padded rows, 24- and 16-bit masks, and block data of pure noise (which reaches BC1's
/// three-colour mode and BC3's six-value mode as often as their other modes).
fn foreign_files() -> Vec<(String, Vec<u8>)> {
    let (w, h) = (37u32, 23u32);
    let px = (w * h) as usize;
    let blocks = (w.div_ceil(4) * h.div_ceil(4)) as usize;
    let mut files = Vec::new();
    let mut add = |name: &str, mut head: Vec<u8>, body: Vec<u8>| {
        head.extend(body);
        files.push((name.to_string(), head));
    };
    add("dx10-rgba8", dx10(w, h, 28), noise(px * 4, 1));
    add("dx10-bgra8", dx10(w, h, 87), noise(px * 4, 2));
    add("dx10-bgrx8", dx10(w, h, 88), noise(px * 4, 3));
    add("legacy-rgba8", header(w, h, DDPF_RGB | DDPF_ALPHAPIXELS, &[0; 4], 32, [0xff, 0xff00, 0xff_0000, 0xff00_0000], None), noise(px * 4, 4));
    add(
        "legacy-bgra8-padded",
        header(w, h, DDPF_RGB | DDPF_ALPHAPIXELS, &[0; 4], 32, [0xff_0000, 0xff00, 0xff, 0xff00_0000], Some(w * 4 + 12)),
        noise((w * 4 + 12) as usize * h as usize, 5),
    );
    add("legacy-rgb24", header(w, h, DDPF_RGB, &[0; 4], 24, [0xff_0000, 0xff00, 0xff, 0], None), noise(px * 3, 6));
    add("legacy-rgb565", header(w, h, DDPF_RGB, &[0; 4], 16, [0xf800, 0x07e0, 0x001f, 0], None), noise(px * 2, 7));
    add("legacy-argb4444", header(w, h, DDPF_RGB | DDPF_ALPHAPIXELS, &[0; 4], 16, [0x0f00, 0x00f0, 0x000f, 0xf000], None), noise(px * 2, 8));
    add("legacy-l8-padded", header(w, h, DDPF_LUMINANCE, &[0; 4], 8, [0xff, 0, 0, 0], Some(w + 3)), noise((w + 3) as usize * h as usize, 9));
    add("legacy-l8a8", header(w, h, DDPF_LUMINANCE | DDPF_ALPHAPIXELS, &[0; 4], 16, [0xff, 0, 0, 0xff00], None), noise(px * 2, 10));
    add("legacy-dxt1-noise", header(w, h, DDPF_FOURCC, b"DXT1", 0, [0; 4], None), noise(blocks * 8, 11));
    add("legacy-dxt3-noise", header(w, h, DDPF_FOURCC, b"DXT3", 0, [0; 4], None), noise(blocks * 16, 12));
    add("legacy-dxt5-noise", header(w, h, DDPF_FOURCC, b"DXT5", 0, [0; 4], None), noise(blocks * 16, 13));
    add("dx10-bc1-noise", dx10(w, h, 71), noise(blocks * 8, 14));
    add("dx10-bc3-noise", dx10(w, h, 77), noise(blocks * 16, 15));
    files
}

/// Every case and its digest, in a fixed order.
fn digests() -> Vec<(String, String)> {
    let mut out = Vec::new();
    // Odd sizes, the sub-block sizes and the one-pixel strips whose mip chains reach the
    // downsampler's edge spans in only one direction.
    let sizes = [(517u32, 389u32), (1, 1), (2, 3), (3, 2), (5, 5), (1, 9), (9, 1), (6, 13)];
    let layouts = [ChannelLayout::Rgba, ChannelLayout::Rgb, ChannelLayout::GrayA, ChannelLayout::Gray];
    // CRAFT_HOSTILE_DIVISOR=N (pull requests) runs every N-th case.
    let step = craft_media_testkit::hostile::divisor();
    let mut index = 0usize;
    for (i, &(w, h)) in sizes.iter().enumerate() {
        for layout in layouts {
            // The large picture in every layout; the small ones in RGBA and gray.
            if i > 0 && !matches!(layout, ChannelLayout::Rgba | ChannelLayout::Gray) {
                continue;
            }
            let img = image(w, h, layout, 0x5eed + i as u64);
            for compression in COMPRESSIONS {
                for mipmaps in [false, true] {
                    index += 1;
                    if !(index - 1).is_multiple_of(step) {
                        continue;
                    }
                    let name = format!("{layout:?}-{w}x{h}-{}-mips{}", compression.name(), u8::from(mipmaps));
                    let opts = EncodeOptions { dds: DdsOptions { compression, mipmaps }, ..Default::default() };
                    let file = encode(&img, Format::Dds, &opts).unwrap_or_else(|e| panic!("{name}: {e}"));
                    out.push((format!("{name}-encode"), sha256_hex(&file)));
                    let back = decode(&file).unwrap_or_else(|e| panic!("{name}: {e}"));
                    out.push((format!("{name}-decode"), image_digest(&back)));
                }
            }
        }
    }
    for (name, file) in foreign_files() {
        index += 1;
        if !(index - 1).is_multiple_of(step) {
            continue;
        }
        let back = decode(&file).unwrap_or_else(|e| panic!("{name}: {e}"));
        out.push((format!("{name}-decode"), image_digest(&back)));
    }
    out
}

#[test]
fn dds_output_matches_the_pinned_digests() {
    let got = digests();
    let pinned = |name: &str| PINNED.iter().find(|(n, _)| *n == name).map(|(_, d)| *d);
    let mismatched: Vec<&str> = got.iter().filter(|(name, digest)| pinned(name) != Some(digest.as_str())).map(|(n, _)| n.as_str()).collect();
    // A sampled run does not compute every pinned case.
    let stale: Vec<&str> = if craft_media_testkit::hostile::divisor() > 1 {
        Vec::new()
    } else {
        PINNED.iter().filter(|(n, _)| !got.iter().any(|(g, _)| g == n)).map(|(n, _)| *n).collect()
    };
    if !mismatched.is_empty() || !stale.is_empty() {
        eprintln!("const PINNED: &[(&str, &str)] = &[");
        for (name, digest) in &got {
            eprintln!("    (\"{name}\", \"{digest}\"),");
        }
        eprintln!("];");
        panic!("{} digests differ or are new: {mismatched:?}; {} pinned cases are gone: {stale:?}", mismatched.len(), stale.len());
    }
}
