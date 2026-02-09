use super::{
    ChapterInfo, NavigationCatalog, ParagraphNavigator, WordSource, WordToken,
    text_utils::{count_words, first_words_excerpt, next_word_at},
};

const PARAGRAPHS_PER_CHAPTER: usize = 2;
const CHAPTER_LABEL_WORDS: usize = 6;

/// Default sample text used until SD-backed content is connected.
pub const DON_QUIJOTE_PARAGRAPHS: [&str; 3] = [
    "En un lugar de la Mancha, de cuyo nombre no quiero acordarme, no ha mucho tiempo que vivía un \
hidalgo de los de lanza en astillero, adarga antigua, rocín flaco y galgo corredor. Una olla de \
algo más vaca que carnero, salpicón las más noches, duelos y quebrantos los sábados, lantejas los \
viernes, algún palomino de añadidura los domingos, consumían las tres partes de su hacienda. El \
resto della concluían sayo de velarte, calzas de velludo para las fiestas, con sus pantuflos de lo \
mesmo, y los días de entresemana se honraba con su vellorí de lo más fino. Tenía en su casa una \
ama que pasaba de los cuarenta, y una sobrina que no llegaba a los veinte, y un mozo de campo y \
plaza, que así ensillaba el rocín como tomaba la podadera. Frisaba la edad de nuestro hidalgo con \
los cincuenta años; era de complexión recia, seco de carnes, enjuto de rostro, gran madrugador y \
amigo de la caza. Quieren decir que tenía el sobrenombre de Quijada, o Quesada, que en esto hay \
alguna diferencia en los autores que deste caso escriben; aunque, por conjeturas verosímiles, se \
deja entender que se llamaba Quejana. Pero esto importa poco a nuestro cuento; basta que en la \
narración dél no se salga un punto de la verdad.",
    "Es, pues, de saber que este sobredicho hidalgo, los ratos que estaba ocioso, que eran los más \
del año, se daba a leer libros de caballerías, con tanta afición y gusto, que olvidó casi de todo \
punto el ejercicio de la caza, y aun la administración de su hacienda. Y llegó a tanto su curiosidad \
y desatino en esto, que vendió muchas hanegas de tierra de sembradura para comprar libros de \
caballerías en que leer, y así, llevó a su casa todos cuantos pudo haber dellos; y de todos, ningunos \
le parecían tan bien como los que compuso el famoso Feliciano de Silva, porque la claridad de su \
prosa y aquellas entricadas razones suyas le parecían de perlas, y más cuando llegaba a leer aquellos \
requiebros y cartas de desafíos, donde en muchas partes hallaba escrito: La razón de la sinrazón \
que a mi razón se hace, de tal manera mi razón enflaquece, que con razón me quejo de la vuestra \
fermosura. Y también cuando leía: …los altos cielos que de vuestra divinidad divinamente con las \
estrellas os fortifican, y os hacen merecedora del merecimiento que merece la vuestra grandeza.",
    "Con estas razones perdía el pobre caballero el juicio, y desvelábase por entenderlas y \
desentrañarles el sentido, que no se lo sacara ni las entendiera el mesmo Aristóteles, si resucitara \
para sólo ello. No estaba muy bien con las heridas que don Belianís daba y recebía, porque se \
imaginaba que, por grandes maestros que le hubiesen curado, no dejaría de tener el rostro y todo \
el cuerpo lleno de cicatrices y señales. Pero, con todo, alababa en su autor aquel acabar su libro con \
la promesa de aquella inacabable aventura, y muchas veces le vino deseo de tomar la pluma y dalle \
fin al pie de la letra, como allí se promete; y sin duda alguna lo hiciera, y aun saliera con ello, si otros \
mayores y continuos pensamientos no se lo estorbaran. Tuvo muchas veces competencia con el cura \
de su lugar -que era hombre docto, graduado en Sigüenza-, sobre cuál había sido mejor caballero: \
Palmerín de Ingalaterra o Amadís de Gaula; mas maese Nicolás, barbero del mesmo pueblo, decía \
que ninguno llegaba al Caballero del Febo, y que si alguno se le podía comparar, era don Galaor, \
hermano de Amadís de Gaula, porque tenía muy acomodada condición para todo; que no era \
caballero melindroso, ni tan llorón como su hermano, y que en lo de la valentía no le iba en zaga.",
];

pub fn default_don_quijote_source() -> StaticWordSource<'static> {
    StaticWordSource::new(&DON_QUIJOTE_PARAGRAPHS)
}

/// Static in-memory content source.
#[derive(Debug, Clone)]
pub struct StaticWordSource<'a> {
    paragraphs: &'a [&'a str],
    paragraph_index: usize,
    paragraph_cursor: usize,
    paragraph_word_index: u16,
    paragraph_word_total: u16,
}

impl<'a> StaticWordSource<'a> {
    pub fn new(paragraphs: &'a [&'a str]) -> Self {
        let mut source = Self {
            paragraphs,
            paragraph_index: 0,
            paragraph_cursor: 0,
            paragraph_word_index: 0,
            paragraph_word_total: 1,
        };
        source.paragraph_word_total = source.compute_current_word_total();
        source
    }

    fn compute_current_word_total(&self) -> u16 {
        if self.paragraphs.is_empty() {
            return 1;
        }

        let count = count_words(self.paragraphs[self.paragraph_index]);
        (count.clamp(1, u16::MAX as usize)) as u16
    }

    fn advance_paragraph(&mut self) {
        if self.paragraphs.is_empty() {
            return;
        }

        self.paragraph_index = (self.paragraph_index + 1) % self.paragraphs.len();
        self.paragraph_cursor = 0;
        self.paragraph_word_index = 0;
        self.paragraph_word_total = self.compute_current_word_total();
    }
}

impl<'a> WordSource for StaticWordSource<'a> {
    type Error = core::convert::Infallible;

    fn reset(&mut self) -> Result<(), Self::Error> {
        self.paragraph_index = 0;
        self.paragraph_cursor = 0;
        self.paragraph_word_index = 0;
        self.paragraph_word_total = self.compute_current_word_total();
        Ok(())
    }

    fn next_word<'b>(&'b mut self) -> Result<Option<WordToken<'b>>, Self::Error> {
        if self.paragraphs.is_empty() {
            return Ok(None);
        }

        let mut attempts = 0usize;

        while attempts < self.paragraphs.len() {
            let paragraph = self.paragraphs[self.paragraph_index];

            if let Some((word, next_cursor)) = next_word_at(paragraph, self.paragraph_cursor) {
                self.paragraph_cursor = next_cursor;
                self.paragraph_word_index = self.paragraph_word_index.saturating_add(1);

                let ends_sentence =
                    word.ends_with('.') || word.ends_with('!') || word.ends_with('?');
                let ends_clause = word.ends_with(',');

                return Ok(Some(WordToken {
                    text: word,
                    ends_sentence,
                    ends_clause,
                }));
            }

            self.advance_paragraph();
            attempts += 1;
        }

        Ok(None)
    }

    fn paragraph_progress(&self) -> (u16, u16) {
        (self.paragraph_word_index, self.paragraph_word_total.max(1))
    }

    fn paragraph_index(&self) -> u16 {
        if self.paragraphs.is_empty() {
            0
        } else {
            (self.paragraph_index + 1) as u16
        }
    }

    fn paragraph_total(&self) -> u16 {
        self.paragraphs.len().clamp(0, u16::MAX as usize) as u16
    }
}

impl ParagraphNavigator for StaticWordSource<'_> {
    fn seek_paragraph(&mut self, paragraph_index: u16) -> Result<(), Self::Error> {
        if self.paragraphs.is_empty() {
            self.paragraph_index = 0;
            self.paragraph_cursor = 0;
            self.paragraph_word_index = 0;
            self.paragraph_word_total = 1;
            return Ok(());
        }

        self.paragraph_index = (paragraph_index as usize).min(self.paragraphs.len() - 1);
        self.paragraph_cursor = 0;
        self.paragraph_word_index = 0;
        self.paragraph_word_total = self.compute_current_word_total();
        Ok(())
    }
}

impl NavigationCatalog for StaticWordSource<'_> {
    fn chapter_count(&self) -> u16 {
        let len = self.paragraphs.len();
        if len == 0 {
            return 1;
        }

        len.div_ceil(PARAGRAPHS_PER_CHAPTER)
            .clamp(1, u16::MAX as usize) as u16
    }

    fn chapter_at(&self, index: u16) -> Option<ChapterInfo<'_>> {
        if self.paragraphs.is_empty() {
            return Some(ChapterInfo {
                label: "Empty",
                start_paragraph: 0,
                paragraph_count: 1,
            });
        }

        let chapter_index = index as usize;
        let chapter_count = self.paragraphs.len().div_ceil(PARAGRAPHS_PER_CHAPTER);
        if chapter_index >= chapter_count {
            return None;
        }

        let start = chapter_index * PARAGRAPHS_PER_CHAPTER;
        let remaining = self.paragraphs.len().saturating_sub(start);
        let count = remaining.min(PARAGRAPHS_PER_CHAPTER);
        let label = first_words_excerpt(self.paragraphs[start], CHAPTER_LABEL_WORDS);

        Some(ChapterInfo {
            label,
            start_paragraph: start as u16,
            paragraph_count: count as u16,
        })
    }

    fn paragraph_preview(&self, paragraph_index: u16) -> Option<&str> {
        self.paragraphs.get(paragraph_index as usize).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::WordSource;

    #[test]
    fn emits_words_and_progress() {
        let paragraphs = ["uno dos", "tres"];
        let mut src = StaticWordSource::new(&paragraphs);

        let first = src.next_word().unwrap().unwrap();
        assert_eq!(first.text, "uno");
        assert_eq!(src.paragraph_progress(), (1, 2));

        let second = src.next_word().unwrap().unwrap();
        assert_eq!(second.text, "dos");
        assert_eq!(src.paragraph_progress(), (2, 2));

        let third = src.next_word().unwrap().unwrap();
        assert_eq!(third.text, "tres");
        assert_eq!(src.paragraph_index(), 2);
    }

    #[test]
    fn punctuation_flags_are_set() {
        let paragraphs = ["hola, mundo. bien?"];
        let mut src = StaticWordSource::new(&paragraphs);

        let a = src.next_word().unwrap().unwrap();
        assert!(a.ends_clause);
        assert!(!a.ends_sentence);

        let b = src.next_word().unwrap().unwrap();
        assert!(b.ends_sentence);

        let c = src.next_word().unwrap().unwrap();
        assert!(c.ends_sentence);
    }
}
