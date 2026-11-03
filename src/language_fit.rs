//! ⚠️ **IMPORTANT:** ⚠️
//! This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
//! conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
//! it before making any changes to this file. After you are done modifying the source code, before
//! you commit your changes, check the Scatterhoard design documents and update them if necessary to
//! keep them current.
//!
//! How well the loaded model represents a language a set declares.
//!
//! A set written in a language its model cannot represent still stores every memory, but each
//! vector is close to arbitrary, so recall by meaning returns noise -- and an agent getting noise
//! from a set it trusts has no way to tell that the model is the cause. **The memories are not at
//! risk**: they are intact, keyword recall works, and recall by meaning returns once a model that
//! covers the language is loaded. What is at risk is the agent's reading of what it gets back, so
//! this measures the fit and says what it means. `recall-is-latin-script-only` in `TRACKING.md`.
//!
//! **The score is how many of ten paraphrase triplets the model ranks first**: for each anchor, its
//! paraphrase against every other sentence written in the same language -- 29 rivals. A rank and
//! not a similarity, because similarity's scale moves with the model: unrelated Russian sentences
//! average 0.87 under `potion-base-32M` and 0.31 under a retrieval model. The number of rivals is
//! part of the measure, not a detail of it: the same Japanese triplets that win 10 of 10 against
//! one unrelated sentence each win 5 of 10 against all 29.
//!
//! The bands come from measurement on 2026-10-04: every non-English language scored 0--4 under the
//! English `potion-base-32M`, and every language 5--10 under `potion-multilingual-128M`.
//! `probe_whether_each_language_ranks_a_paraphrase_first` in `tests/probes.rs` reproduces it.

use crate::embedder::{Embedded, Embedder, EmbedderError};
use crate::embedding::cosine;

/// How many triplets a language is scored on, and so the top of the scale.
pub const TRIPLETS_PER_LANGUAGE: usize = 10;

/// An anchor, a paraphrase of it, and an unrelated sentence.
pub type Triplet = [&'static str; 3];

/// One language's built-in test sentences.
#[derive(Debug)]
pub struct Sentences {
    /// The language subtag a `hoard-lang-` keyword names it by, in the caseless form keywords are
    /// matched in.
    pub tag: &'static str,
    /// The language's English name, for notes a person will read.
    pub name: &'static str,
    /// The script the sentences are written in, where the language has more than one in use. A tag
    /// naming a different script is not scored with these.
    pub script: Option<&'static str>,
    /// Ten triplets, in the same order in every language, so the `n`th says the same thing in each.
    pub triplets: [Triplet; TRIPLETS_PER_LANGUAGE],
}

/// The languages the server carries sentences for.
///
/// **The English paraphrases avoid the anchors' words; the translations were written by an LLM**,
/// and not every language can avoid a shared word or character -- Chinese 车, 会 and 纸 among them
/// -- so a paraphrase can be helped by surface overlap, more in some languages than others. No
/// unrelated sentence shares a topic with any anchor, because every anchor is scored against all of
/// them. A language not listed here is scored from sentences the agent supplies.
pub const BUILT_IN: [Sentences; 8] = [
    Sentences {
        tag: "en",
        name: "English",
        script: None,
        triplets: [
            [
                "should we rent our own car",
                "is it worth hiring a vehicle for the trip",
                "the soup needs more salt",
            ],
            [
                "the meeting was moved to next Tuesday",
                "they postponed the gathering by a week",
                "my bicycle has a flat tire",
            ],
            [
                "the library closes early on Fridays",
                "at the end of the week the reading room shuts sooner",
                "we planted tomatoes in the garden",
            ],
            [
                "the printer on the second floor is out of paper",
                "there is no paper left in the upstairs copier",
                "she paints portraits of her neighbours",
            ],
            [
                "my sister is learning to play the violin",
                "my sibling has started taking fiddle lessons",
                "the cat sleeps on the windowsill",
            ],
            [
                "it rained all weekend so we stayed inside",
                "the wet Saturday and Sunday kept us indoors",
                "he forgot his password again",
            ],
            [
                "the doctor told him to drink more water",
                "his physician advised increasing his fluid intake",
                "the museum has a new dinosaur skeleton",
            ],
            [
                "we ran out of coffee this morning",
                "there was nothing left to brew at breakfast",
                "the bridge was painted bright red",
            ],
            [
                "the children are afraid of the dark",
                "the kids get scared when the lights go out",
                "taxes are due in April",
            ],
            [
                "she sold her old house and moved to the coast",
                "after parting with her former home she relocated by the sea",
                "this keyboard is too loud",
            ],
        ],
    },
    Sentences {
        tag: "es",
        name: "Spanish",
        script: None,
        triplets: [
            [
                "¿deberíamos alquilar nuestro propio coche?",
                "¿vale la pena contratar un vehículo para el viaje?",
                "la sopa necesita más sal",
            ],
            [
                "la reunión se cambió al próximo martes",
                "aplazaron el encuentro una semana",
                "mi bicicleta tiene una rueda pinchada",
            ],
            [
                "la biblioteca cierra temprano los viernes",
                "al final de la semana la sala de lectura abre menos horas",
                "plantamos tomates en el huerto",
            ],
            [
                "la impresora del segundo piso no tiene papel",
                "en la fotocopiadora de arriba ya no quedan hojas",
                "ella pinta retratos de sus vecinos",
            ],
            [
                "mi hermana está aprendiendo a tocar el violín",
                "a la hija de mis padres le están enseñando un instrumento de cuerda",
                "el gato duerme en el alféizar",
            ],
            [
                "llovió todo el fin de semana y nos quedamos en casa",
                "el sábado y el domingo húmedos nos tuvieron bajo techo",
                "él olvidó su contraseña otra vez",
            ],
            [
                "el médico le dijo que bebiera más agua",
                "su doctor le aconsejó aumentar la ingesta de líquidos",
                "el museo tiene un nuevo esqueleto de dinosaurio",
            ],
            [
                "esta mañana se nos acabó el café",
                "en el desayuno no quedaba nada para preparar",
                "pintaron el puente de rojo intenso",
            ],
            [
                "los niños tienen miedo a la oscuridad",
                "los pequeños se asustan cuando se apagan las luces",
                "los impuestos vencen en abril",
            ],
            [
                "vendió su vieja casa y se mudó a la costa",
                "tras deshacerse de su antiguo hogar se instaló junto al mar",
                "este teclado hace demasiado ruido",
            ],
        ],
    },
    Sentences {
        tag: "de",
        name: "German",
        script: None,
        triplets: [
            [
                "sollten wir unser eigenes Auto mieten",
                "lohnt es sich, für die Reise ein Fahrzeug zu nehmen",
                "die Suppe braucht mehr Salz",
            ],
            [
                "das Treffen wurde auf nächsten Dienstag verlegt",
                "sie haben die Zusammenkunft um eine Woche verschoben",
                "mein Fahrrad hat einen platten Reifen",
            ],
            [
                "die Bibliothek schließt freitags früh",
                "am Ende der Woche macht der Lesesaal eher zu",
                "wir haben Tomaten im Garten gepflanzt",
            ],
            [
                "der Drucker im zweiten Stock hat kein Papier mehr",
                "im Kopierer oben sind keine Blätter mehr",
                "sie malt Porträts ihrer Nachbarn",
            ],
            [
                "meine Schwester lernt Geige spielen",
                "die Tochter meiner Eltern nimmt jetzt Violinunterricht",
                "die Katze schläft auf der Fensterbank",
            ],
            [
                "es hat das ganze Wochenende geregnet, also blieben wir drinnen",
                "der nasse Samstag und Sonntag hielten uns im Haus",
                "er hat sein Passwort schon wieder vergessen",
            ],
            [
                "der Arzt sagte ihm, er solle mehr Wasser trinken",
                "sein Mediziner riet ihm, mehr Flüssigkeit zu sich zu nehmen",
                "das Museum hat ein neues Dinosaurierskelett",
            ],
            [
                "heute Morgen ist uns der Kaffee ausgegangen",
                "zum Frühstück war nichts mehr zum Aufbrühen da",
                "die Brücke wurde knallrot gestrichen",
            ],
            [
                "die Kinder haben Angst im Dunkeln",
                "die Kleinen fürchten sich, wenn das Licht ausgeht",
                "die Steuern sind im April fällig",
            ],
            [
                "sie hat ihr altes Haus verkauft und ist an die Küste gezogen",
                "nachdem sie ihr früheres Heim abgegeben hatte, wohnt sie jetzt am Meer",
                "diese Tastatur ist zu laut",
            ],
        ],
    },
    Sentences {
        tag: "fr",
        name: "French",
        script: None,
        triplets: [
            [
                "devrions-nous louer notre propre voiture",
                "est-ce que ça vaut la peine de prendre un véhicule pour le voyage",
                "la soupe a besoin de plus de sel",
            ],
            [
                "la réunion a été déplacée à mardi prochain",
                "ils ont reporté le rendez-vous d'une semaine",
                "mon vélo a un pneu crevé",
            ],
            [
                "la bibliothèque ferme tôt le vendredi",
                "à la fin de la semaine, la salle de lecture ouvre moins longtemps",
                "nous avons planté des tomates dans le jardin",
            ],
            [
                "l'imprimante du deuxième étage n'a plus de papier",
                "il ne reste aucune feuille dans le photocopieur d'en haut",
                "elle peint des portraits de ses voisins",
            ],
            [
                "ma sœur apprend à jouer du violon",
                "la fille de mes parents prend désormais des cours d'instrument à cordes",
                "le chat dort sur le rebord de la fenêtre",
            ],
            [
                "il a plu tout le week-end, alors nous sommes restés à l'intérieur",
                "le samedi et le dimanche mouillés nous ont gardés à la maison",
                "il a encore oublié son mot de passe",
            ],
            [
                "le médecin lui a dit de boire plus d'eau",
                "son docteur lui a conseillé d'augmenter sa consommation de liquides",
                "le musée a un nouveau squelette de dinosaure",
            ],
            [
                "nous n'avions plus de café ce matin",
                "au petit-déjeuner, il ne restait rien à infuser",
                "le pont a été peint en rouge vif",
            ],
            [
                "les enfants ont peur du noir",
                "les petits s'effraient quand les lumières s'éteignent",
                "les impôts sont à payer en avril",
            ],
            [
                "elle a vendu sa vieille maison et s'est installée sur la côte",
                "après s'être séparée de son ancien logement, elle vit au bord de la mer",
                "ce clavier est trop bruyant",
            ],
        ],
    },
    Sentences {
        tag: "ru",
        name: "Russian",
        script: None,
        triplets: [
            [
                "должны ли мы арендовать собственную машину",
                "стоит ли взять транспорт на время поездки",
                "супу нужно больше соли",
            ],
            [
                "встречу перенесли на следующий вторник",
                "собрание отложили на неделю",
                "у моего велосипеда спустило колесо",
            ],
            [
                "по пятницам библиотека закрывается рано",
                "в конце недели читальный зал работает меньше",
                "мы посадили помидоры в огороде",
            ],
            [
                "в принтере на втором этаже закончилась бумага",
                "в копировальном аппарате наверху не осталось листов",
                "она рисует портреты своих соседей",
            ],
            [
                "моя сестра учится играть на скрипке",
                "дочь моих родителей начала брать уроки игры на смычковом инструменте",
                "кошка спит на подоконнике",
            ],
            [
                "все выходные шёл дождь, поэтому мы сидели дома",
                "мокрые суббота и воскресенье удержали нас под крышей",
                "он снова забыл свой пароль",
            ],
            [
                "врач сказал ему пить больше воды",
                "доктор посоветовал ему увеличить потребление жидкости",
                "в музее появился новый скелет динозавра",
            ],
            [
                "сегодня утром у нас кончился кофе",
                "к завтраку нечего было заварить",
                "мост покрасили в ярко-красный цвет",
            ],
            [
                "дети боятся темноты",
                "малыши пугаются, когда гаснет свет",
                "налоги нужно заплатить в апреле",
            ],
            [
                "она продала свой старый дом и переехала на побережье",
                "расставшись с прежним жильём, она поселилась у моря",
                "эта клавиатура слишком громкая",
            ],
        ],
    },
    Sentences {
        tag: "zh",
        name: "Chinese",
        script: Some("hans"),
        triplets: [
            [
                "我们应该租一辆自己的车吗",
                "为这次旅行雇一辆车值得吗",
                "这个汤需要多放点盐",
            ],
            [
                "会议改到了下周二",
                "他们把聚会推迟了一个星期",
                "我的自行车爆胎了",
            ],
            [
                "图书馆每周五很早关门",
                "周末前阅览室开放时间较短",
                "我们在花园里种了西红柿",
            ],
            [
                "二楼的打印机没纸了",
                "楼上的复印机一张也不剩了",
                "她给邻居们画肖像",
            ],
            [
                "我姐姐在学拉小提琴",
                "我父母的女儿开始上弦乐课了",
                "猫睡在窗台上",
            ],
            [
                "整个周末都在下雨，所以我们待在家里",
                "潮湿的星期六和星期天让我们没出门",
                "他又忘了密码",
            ],
            [
                "医生让他多喝水",
                "大夫建议他增加液体摄入",
                "博物馆新添了一具恐龙骨架",
            ],
            [
                "今天早上咖啡喝完了",
                "早餐时已经没有东西可以冲泡了",
                "那座桥被漆成了鲜红色",
            ],
            ["孩子们怕黑", "灯一灭小家伙们就害怕", "四月份要交税"],
            [
                "她卖掉了旧房子，搬到了沿海",
                "她出手了以前的住所，在海边安了家",
                "这个键盘太吵了",
            ],
        ],
    },
    Sentences {
        tag: "hi",
        name: "Hindi",
        script: None,
        triplets: [
            [
                "क्या हमें अपनी कार किराए पर लेनी चाहिए",
                "क्या यात्रा के लिए गाड़ी लेना ठीक रहेगा",
                "सूप में और नमक चाहिए",
            ],
            [
                "बैठक अगले मंगलवार को कर दी गई",
                "उन्होंने सभा को एक हफ़्ते के लिए टाल दिया",
                "मेरी साइकिल का टायर पंचर हो गया",
            ],
            [
                "पुस्तकालय शुक्रवार को जल्दी बंद हो जाता है",
                "हफ़्ते के आख़िर में वाचनालय कम समय खुलता है",
                "हमने बगीचे में टमाटर लगाए",
            ],
            [
                "दूसरी मंज़िल के प्रिंटर में कागज़ ख़त्म हो गया है",
                "ऊपर वाली फ़ोटोकॉपी मशीन में एक भी पन्ना नहीं बचा",
                "वह अपने पड़ोसियों के चित्र बनाती है",
            ],
            [
                "मेरी बहन वायलिन बजाना सीख रही है",
                "मेरे माता-पिता की बेटी ने तार वाले वाद्य की कक्षाएँ शुरू की हैं",
                "बिल्ली खिड़की की चौखट पर सोती है",
            ],
            [
                "पूरे सप्ताहांत बारिश हुई इसलिए हम अंदर रहे",
                "भीगे शनिवार और रविवार ने हमें घर में ही रोके रखा",
                "वह फिर से अपना पासवर्ड भूल गया",
            ],
            [
                "डॉक्टर ने उससे ज़्यादा पानी पीने को कहा",
                "चिकित्सक ने उसे तरल पदार्थ बढ़ाने की सलाह दी",
                "संग्रहालय में डायनासोर का एक नया कंकाल आया है",
            ],
            [
                "आज सुबह हमारी कॉफ़ी ख़त्म हो गई",
                "नाश्ते के समय बनाने के लिए कुछ नहीं बचा था",
                "पुल को चटक लाल रंग से रंगा गया",
            ],
            [
                "बच्चे अँधेरे से डरते हैं",
                "बत्ती बुझते ही छोटे घबरा जाते हैं",
                "कर अप्रैल में भरने हैं",
            ],
            [
                "उसने अपना पुराना घर बेचकर तट पर रहना शुरू किया",
                "पिछला मकान छोड़कर वह समुद्र के किनारे बस गई",
                "यह कीबोर्ड बहुत शोर करता है",
            ],
        ],
    },
    Sentences {
        tag: "ja",
        name: "Japanese",
        script: None,
        triplets: [
            [
                "自分の車を借りるべきですか",
                "旅行のために乗り物を手配する価値はありますか",
                "スープにもっと塩が必要です",
            ],
            [
                "会議は来週の火曜日に変更された",
                "彼らは集まりを一週間延期した",
                "自転車のタイヤがパンクした",
            ],
            [
                "図書館は金曜日は早く閉まる",
                "週末前には閲覧室の開いている時間が短い",
                "私たちは庭にトマトを植えた",
            ],
            [
                "二階のプリンターの紙が切れている",
                "上の階のコピー機には用紙が一枚も残っていない",
                "彼女は近所の人たちの肖像画を描く",
            ],
            [
                "妹はバイオリンを習っている",
                "両親の娘が弦楽器のレッスンを受け始めた",
                "猫が窓辺で眠っている",
            ],
            [
                "週末ずっと雨だったので家にいた",
                "湿った土曜と日曜のせいで外出しなかった",
                "彼はまたパスワードを忘れた",
            ],
            [
                "医者は彼にもっと水を飲むように言った",
                "主治医は水分の摂取を増やすよう勧めた",
                "博物館に新しい恐竜の骨格が展示された",
            ],
            [
                "今朝コーヒーがなくなった",
                "朝食の時に淹れるものが何も残っていなかった",
                "その橋は真っ赤に塗られた",
            ],
            [
                "子供たちは暗闇を怖がる",
                "明かりが消えると小さい子はおびえる",
                "税金は四月に納める",
            ],
            [
                "彼女は古い家を売って海岸に引っ越した",
                "以前の住まいを手放して、海のそばに移り住んだ",
                "このキーボードはうるさすぎる",
            ],
        ],
    },
];

/// The built-in sentences for a declared language tag, if there are any.
///
/// Matched on the language subtag, so `en-gb` is scored as `en`. A script subtag must agree with
/// the script the sentences are written in, where that is recorded: `zh-hant` is not scored with
/// simplified Chinese, because that would measure a script the set does not use.
#[must_use]
pub fn built_in(tag: &str) -> Option<&'static Sentences> {
    let (language, script) = language_and_script(tag);
    BUILT_IN.iter().find(|sentences| {
        sentences.tag == language && scripts_agree(script.as_deref(), sentences.script)
    })
}

/// The part of a language tag that changes what a model sees: its language and, where one is
/// given, its script -- lowercased, with any region dropped.
///
/// **Region is ignored because it barely moves a model**, which represents "colour" as readily as
/// "color"; **script is kept because it changes everything**, as hieroglyphs and their Latin
/// transliteration do. So `en-US` and `en-gb` give `("en", None)`, and `egy-Latn` gives
/// `("egy", Some("latn"))`. A script subtag is four letters, which is how BCP 47 tells it from a
/// two-letter or three-digit region.
#[must_use]
pub fn language_and_script(tag: &str) -> (String, Option<String>) {
    let mut subtags = tag.split('-');
    let language = subtags.next().unwrap_or_default().to_ascii_lowercase();
    let script = subtags
        .next()
        .filter(|subtag| subtag.len() == 4 && subtag.chars().all(|c| c.is_ascii_alphabetic()))
        .map(str::to_ascii_lowercase);
    (language, script)
}

/// Whether two scripts can be the same text: equal, or either unstated.
#[must_use]
pub fn scripts_agree(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left == right,
        _ => true,
    }
}

/// The triplet a sentence-set memory carries: the first JSON array in its text, which must be
/// three sentences that are not blank.
///
/// **The first array, read as JSON**, so prose before it -- the line a reviewer and an index read --
/// is allowed, any punctuation inside a sentence is safe, and `\uXXXX` escapes are understood.
///
/// # Errors
///
/// What is wrong with the text, phrased for the caller who wrote it.
pub fn triplet_in(text: &str) -> Result<[String; 3], String> {
    let start = text
        .find('[')
        .ok_or_else(|| "it holds no JSON array".to_owned())?;
    let array: Vec<String> = serde_json::Deserializer::from_str(&text[start..])
        .into_iter::<Vec<String>>()
        .next()
        .ok_or_else(|| "it holds no JSON array".to_owned())?
        .map_err(|error| format!("its array is not a list of sentences: {error}"))?;
    let triplet: [String; 3] = array.try_into().map_err(|array: Vec<String>| {
        format!("its array holds {} sentences, not three", array.len())
    })?;
    if triplet.iter().any(|sentence| sentence.trim().is_empty()) {
        return Err("one of its sentences is blank".to_owned());
    }
    Ok(triplet)
}

/// How many triplets the model ranks first.
///
/// For each anchor, its paraphrase is ranked against every other sentence in `triplets`, and wins
/// only by scoring strictly above all of them. **A tie loses**: a model that scores everything
/// alike has told nothing apart, and that is exactly the collapse this exists to catch. An anchor
/// or paraphrase the model cannot represent loses its triplet; a rival it cannot represent has no
/// position and outranks nothing.
///
/// The bands assume [`TRIPLETS_PER_LANGUAGE`] triplets, so a caller scoring any other number is
/// measuring something the bands do not describe.
///
/// # Errors
///
/// [`EmbedderError`] if the model cannot embed at all.
pub fn score(embedder: &dyn Embedder, triplets: &[[&str; 3]]) -> Result<u8, EmbedderError> {
    let texts: Vec<String> = triplets.iter().flatten().map(|s| (*s).to_owned()).collect();
    let vectors: Vec<Option<Vec<f32>>> = embedder
        .embed_each(&texts)?
        .into_iter()
        .map(|outcome| match outcome {
            Embedded::Vector(vector) => Some(vector.components().to_vec()),
            Embedded::Unrepresentable => None,
        })
        .collect();
    let mut first = 0u8;
    for index in 0..triplets.len() {
        let (anchor, paraphrase) = (3 * index, 3 * index + 1);
        let (Some(anchor_vector), Some(paraphrase_vector)) =
            (&vectors[anchor], &vectors[paraphrase])
        else {
            continue;
        };
        let target = cosine(anchor_vector, paraphrase_vector);
        let beaten = vectors
            .iter()
            .enumerate()
            .filter(|(position, _)| *position != anchor && *position != paraphrase)
            .filter_map(|(_, vector)| vector.as_ref())
            .any(|rival| cosine(anchor_vector, rival) >= target);
        if !beaten {
            first += 1;
        }
    }
    Ok(first)
}

/// What a score means for recall in that language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    /// 7 to 10: recall by meaning works.
    Searchable,
    /// 4 to 6: recall works sometimes, and a model built for the language would likely do better.
    Unreliable,
    /// 0 to 3: the model cannot tell this language's sentences apart by meaning.
    Unsearchable,
}

impl Band {
    /// The band a score falls in.
    #[must_use]
    pub fn of(score: u8) -> Self {
        match score {
            7.. => Self::Searchable,
            4..=6 => Self::Unreliable,
            _ => Self::Unsearchable,
        }
    }

    /// What to tell the agent about a language in this band, or nothing when recall works.
    ///
    /// **The middle band says to change models rather than that recall partly works**, because a
    /// score of 4 or 5 can come from a model that has not separated the language at all and was
    /// helped by shared characters -- Japanese scored 4 under the English model with its paraphrases
    /// behind their rivals on average. *Unreliable, a better model exists* is true either way.
    #[must_use]
    pub fn note(self, language: &str) -> Option<String> {
        match self {
            Self::Searchable => None,
            Self::Unreliable => Some(format!(
                "Recall by meaning in {language} is unreliable under this set's embedding model: \
                 related memories may rank below unrelated ones. A model built for {language} \
                 would likely do better, possibly at the cost of other languages written in the \
                 same set -- tell the user, since which languages the set needs is theirs to weigh."
            )),
            Self::Unsearchable => Some(format!(
                "This set's embedding model cannot search {language} by meaning: recall will \
                 return memories unrelated to the query. The memories are intact, keyword recall \
                 and hoard_lookup work, and recall by meaning returns once a model that covers \
                 {language} is loaded -- tell the user this set needs one."
            )),
        }
    }
}
