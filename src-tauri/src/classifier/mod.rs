use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuestionType {
    Technical,
    Star,
    Architecture,
    Trap,
    None,
}

impl QuestionType {
    pub fn as_str(&self) -> &'static str {
        match self {
            QuestionType::Technical => "technical",
            QuestionType::Star => "star",
            QuestionType::Architecture => "architecture",
            QuestionType::Trap => "trap",
            QuestionType::None => "none",
        }
    }
}

const IMPERATIVE_TRIGGERS: &[&str] = &[
    "cuéntame",
    "dime",
    "descríbeme",
    "explícame",
    "camínenme por",
    "háblame",
    "compárteme",
    "platícame",
    "nárrame",
    "tell me",
    "describe",
    "explain",
    "walk me through",
    "talk me through",
    "share",
    "give me an example",
    "give me an example of",
    "give me an instance of",
    "could you",
    "would you",
    "can you",
    "will you",
    "do you",
    "have you",
    "has you",
    "are you able to",
    "podrías",
    "puedes",
    "puedas",
    "harías",
    "harias",
    "me cuentas",
    "me describes",
    "me explicas",
    "me hablas",
    "me compartes",
    "cuéntanos",
    "cuentanos",
    "dinos",
    "dinos",
    "explícanos",
    "explicanos",
    "cuéntame de una vez",
    "cuéntame sobre",
    "dime sobre",
    "háblame de",
    "walk us through",
    "run me through",
    "run us through",
    "paint me a picture",
];

const QUESTION_START_TRIGGERS: &[&str] = &[
    "what", "which", "who", "where", "when", "why", "how", "are", "is", "can", "could", "would",
    "will", "do", "does", "did", "have", "has", "should", "shall", "qué", "cuál", "cuáles",
    "cuando", "cuándo", "dónde", "donde", "por qué", "porque", "porqué", "quién", "quien", "cómo",
    "como", "es", "está", "esta", "puede", "podría", "debería", "tiene", "ha", "hace", "será",
    "seria",
];

const EXCLUSION_LIST: &[&str] = &[
    "cómo estás",
    "cómo está",
    "me escuchas",
    "me oyes",
    "how are you",
    "can you hear me",
    "are you there",
    "how's it going",
    "you there",
    "you still there",
];

const TECHNICAL_KEYWORDS: &[&str] = &[
    "error",
    "código",
    "code",
    "debug",
    "implementaste",
    "implement",
    "bug",
    "test",
    "prueba",
    "performance",
    "rendimiento",
    "optimiz",
    "api",
    "endpoint",
    "database",
    "base de datos",
    "algoritmo",
    "algorithm",
    "estructura de datos",
    "data structure",
    "complejidad",
    "complexity",
    "memoria",
    "memory",
    "concurrencia",
    "concurrency",
    "thread",
    "hilo",
    "async",
    "asíncrono",
    "framework",
    "librería",
    "library",
    "dependencia",
    "dependency",
    "tecnología",
    "technology",
    "stack",
    "lenguaje",
    "language",
    "cómo lo hiciste",
    "cómo lo resolviste",
    "cómo implementaste",
    "how did you",
    "what technology",
    "what library",
];

const STAR_KEYWORDS: &[&str] = &[
    "lideraste",
    "lead",
    "liderazgo",
    "leadership",
    "equipo",
    "team",
    "conflicto",
    "conflict",
    "situación",
    "situation",
    "desacuerdo",
    "disagreement",
    "negociación",
    "negotiation",
    "comunicación",
    "communication",
    "colaboración",
    "collaboration",
    "retroalimentación",
    "feedback",
    "mentor",
    "mentoring",
    "delegación",
    "delegation",
    "presión",
    "pressure",
    "fecha límite",
    "deadline",
    "stakeholder",
    "cliente difícil",
    "difficult client",
    "fracaso",
    "failure",
    "error",
    "mistake",
    "aprendiste",
    "learned",
    "creciste",
    "grew",
    "momento",
    "experiencia",
    "experience",
    "vez que",
    "time when",
    "cuéntame de una vez",
    "tell me about a time",
    "dame un ejemplo",
    "give me an example",
    "cómo manejaste",
    "how did you handle",
    "cómo resolviste",
    "cómo manejas",
    "cómo manejó",
];

const ARCHITECTURE_KEYWORDS: &[&str] = &[
    "arquitectura",
    "architecture",
    "escalabilidad",
    "scalability",
    "diseño",
    "design",
    "patrón",
    "pattern",
    "microservicios",
    "microservices",
    "monolito",
    "monolith",
    "eventos",
    "events",
    "cqrs",
    "event sourcing",
    "hexagonal",
    "clean architecture",
    "ddd",
    "domain driven",
    "capas",
    "layers",
    "modular",
    "módulos",
    "acoplamiento",
    "coupling",
    "cohesión",
    "cohesion",
    "solid",
    "principios",
    "principle",
    "distribuido",
    "distributed",
    "alta disponibilidad",
    "high availability",
    "disponibilidad",
    "availability",
    "tolerancia a fallos",
    "fault tolerance",
    "diagrama",
    "componentes",
    "components",
    "cómo diseñaste",
    "how did you design",
    "cómo modelaste",
    "how would you design",
    "diseña",
    "design a",
];

const TRAP_KEYWORDS: &[&str] = &[
    "defecto",
    "defect",
    "debilidad",
    "weakness",
    "fallo",
    "fail",
    "crítica",
    "criticism",
    "crítica",
    "critique",
    "peor",
    "worst",
    "mayor error",
    "biggest mistake",
    "mayor fracaso",
    "biggest failure",
    "despedir",
    "fire",
    "despedido",
    "fired",
    "odias",
    "hate",
    "detestas",
    "detest",
    "no te gusta",
    "don't like",
    "qué harías diferente",
    "what would you do differently",
    "qué cambiarías",
    "what would you change",
    "por qué deberíamos contratarte",
    "why should we hire you",
    "por qué te fuiste",
    "why did you leave",
    "arrepientes",
    "regret",
    "arrepentimiento",
    "frustr",
    "frustration",
    "frustración",
    "fracaso profesional",
    "professional failure",
    "debilidad profesional",
    "professional weakness",
    "punto débil",
    "weak point",
    "peor proyecto",
    "worst project",
    "experiencia negativa",
    "negative experience",
    "qué aprendiste de tus errores",
    "what did you learn from your mistakes",
    "retroalimentación negativa",
    "negative feedback",
    "situación incómoda",
    "awkward situation",
];

pub fn classify(text: &str) -> QuestionType {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return QuestionType::None;
    }

    let lower = trimmed.to_lowercase();

    // Exclusion list: small talk → None
    for phrase in EXCLUSION_LIST {
        if lower.contains(phrase) {
            return QuestionType::None;
        }
    }

    // Check if it's a question: question mark, imperative verb at start, or
    // a clear question word / auxiliary at the beginning.
    let has_question_mark = lower.contains('?');

    // Compute body once — text after a potential leading filler word.
    let body = lower.splitn(2, ' ').nth(1).unwrap_or(&lower);
    let starts_with_imperative = IMPERATIVE_TRIGGERS
        .iter()
        .any(|&trigger| lower.starts_with(trigger) || body.starts_with(trigger));

    let first_word = lower.split_whitespace().next().unwrap_or("");
    let starts_as_question = QUESTION_START_TRIGGERS
        .iter()
        .any(|&trigger| first_word == trigger);

    if !has_question_mark && !starts_with_imperative && !starts_as_question {
        return QuestionType::None;
    }

    // Type classification by keyword density
    let mut technical_score = 0;
    let mut behavioral_score = 0;
    let mut architecture_score = 0;
    let mut trap_score = 0;

    for kw in TECHNICAL_KEYWORDS {
        if lower.contains(kw) {
            technical_score += 1;
        }
    }
    for kw in STAR_KEYWORDS {
        if lower.contains(kw) {
            behavioral_score += 1;
        }
    }
    for kw in ARCHITECTURE_KEYWORDS {
        if lower.contains(kw) {
            architecture_score += 1;
        }
    }
    for kw in TRAP_KEYWORDS {
        if lower.contains(kw) {
            trap_score += 1;
        }
    }

    // If no keywords matched, try to infer type from known question patterns
    if technical_score == 0 && behavioral_score == 0 && architecture_score == 0 && trap_score == 0 {
        // Technical questions often ask about implementation details
        if lower.contains("cómo")
            || lower.contains("how")
            || lower.contains("qué tecnología")
            || lower.contains("qué herramienta")
            || lower.contains("what tool")
            || lower.contains("what technology")
            || lower.contains("what language")
        {
            technical_score += 1;
        }
        // Star questions often ask about experiences
        if lower.contains("cuándo")
            || lower.contains("when")
            || lower.contains("dime")
            || (lower.contains("tell") && lower.contains("about"))
            || lower.contains("experiencia")
            || lower.contains("experience")
        {
            behavioral_score += 1;
        }
    }

    // "Cuéntame", "dime", "tell me about", "walk me through" patterns
    // that ask about experiences are strongly behavioral even without
    // explicit keywords.
    let is_experience_question = lower.contains("cuéntame")
        || lower.contains("tell me about")
        || (lower.contains("walk me through") && !lower.contains("code"))
        || lower.contains("dime")
        || lower.contains("háblame")
        || lower.contains("give me an example");
    if is_experience_question && behavioral_score == 0 && architecture_score == 0 && trap_score == 0
    {
        behavioral_score = 1;
    }

    // Highest score wins
    let max_score = *[
        technical_score,
        behavioral_score,
        architecture_score,
        trap_score,
    ]
    .iter()
    .max()
    .unwrap_or(&0);

    if max_score == 0 {
        // Fallback: treat it as a detected question but unknown type
        return QuestionType::Technical;
    }

    if trap_score == max_score && trap_score > 0 {
        return QuestionType::Trap;
    }
    if architecture_score == max_score && architecture_score > 0 {
        return QuestionType::Architecture;
    }
    if behavioral_score == max_score && behavioral_score > 0 {
        return QuestionType::Star;
    }

    QuestionType::Technical
}

#[tauri::command]
pub fn classify_text(text: String) -> QuestionType {
    classify(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Question detection: explicit question mark
    // -----------------------------------------------------------------------

    #[test]
    fn detects_technical_question_with_mark() {
        assert_eq!(
            classify("¿Cómo implementaste la caché?"),
            QuestionType::Technical
        );
    }

    #[test]
    fn detects_english_technical_with_mark() {
        assert_eq!(
            classify("How did you implement the cache?"),
            QuestionType::Technical
        );
    }

    // -----------------------------------------------------------------------
    // Question detection: imperative verb without question mark
    // -----------------------------------------------------------------------

    #[test]
    fn detects_behavioral_without_mark_cuentame() {
        assert_eq!(
            classify("Cuéntame de una vez que tuviste un conflicto en el equipo"),
            QuestionType::Star
        );
    }

    #[test]
    fn detects_behavioral_without_mark_dime() {
        assert_eq!(
            classify("Dime de una situación donde lideraste un equipo difícil"),
            QuestionType::Star
        );
    }

    #[test]
    fn detects_technical_without_mark_describe() {
        assert_eq!(
            classify("Descríbeme cómo implementaste un sistema distribuido"),
            QuestionType::Technical
        );
    }

    #[test]
    fn detects_architecture_with_explicate() {
        assert_eq!(
            classify("Explícame cómo diseñaste la arquitectura del sistema"),
            QuestionType::Architecture
        );
    }

    #[test]
    fn detects_behavioral_walk_me_through() {
        assert_eq!(
            classify("Walk me through a time you had a conflict in your team"),
            QuestionType::Star
        );
    }

    // -----------------------------------------------------------------------
    // Exclusion list
    // -----------------------------------------------------------------------

    #[test]
    fn excludes_small_talk_como_estas() {
        assert_eq!(classify("¿Cómo estás?"), QuestionType::None);
    }

    #[test]
    fn excludes_small_talk_me_escuchas() {
        assert_eq!(classify("¿Me escuchas bien?"), QuestionType::None);
    }

    #[test]
    fn excludes_english_small_talk() {
        assert_eq!(classify("How are you?"), QuestionType::None);
    }

    #[test]
    fn excludes_can_you_hear_me() {
        assert_eq!(classify("Can you hear me?"), QuestionType::None);
    }

    #[test]
    fn excludes_are_you_there() {
        assert_eq!(classify("Are you there?"), QuestionType::None);
    }

    // -----------------------------------------------------------------------
    // Type classification by keywords
    // -----------------------------------------------------------------------

    #[test]
    fn classifies_technical_by_keyword() {
        assert_eq!(
            classify("¿Qué tecnología usaste para implementar el caché?"),
            QuestionType::Technical
        );
    }

    #[test]
    fn classifies_behavioral_by_keyword() {
        assert_eq!(
            classify("¿Cómo manejaste un conflicto en tu equipo?"),
            QuestionType::Star
        );
    }

    #[test]
    fn classifies_architecture_by_keyword() {
        assert_eq!(
            classify("¿Cómo diseñaste la escalabilidad del sistema?"),
            QuestionType::Architecture
        );
    }

    #[test]
    fn classifies_trap_by_keyword() {
        assert_eq!(classify("¿Cuál es tu mayor debilidad?"), QuestionType::Trap);
    }

    // -----------------------------------------------------------------------
    // No question → None
    // -----------------------------------------------------------------------

    #[test]
    fn statement_returns_none() {
        assert_eq!(
            classify("Implementé la caché con Redis"),
            QuestionType::None
        );
    }

    #[test]
    fn empty_returns_none() {
        assert_eq!(classify(""), QuestionType::None);
    }

    #[test]
    fn whitespace_returns_none() {
        assert_eq!(classify("   "), QuestionType::None);
    }

    // -----------------------------------------------------------------------
    // Imperative trigger with leading filler words
    // -----------------------------------------------------------------------

    #[test]
    fn detects_imperative_after_leading_word() {
        assert_eq!(
            classify("Bueno cuéntame de un proyecto difícil"),
            QuestionType::Star
        );
    }

    #[test]
    fn detects_imperative_after_so() {
        assert_eq!(
            classify("So tell me about a time you had a conflict"),
            QuestionType::Star
        );
    }

    // -----------------------------------------------------------------------
    // Edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn question_mark_without_keywords_defaults_technical() {
        // When no keywords match but there's a question mark,
        // the heuristic defaults to Technical
        assert_eq!(
            classify("¿Qué piensas sobre el futuro del desarrollo?"),
            QuestionType::Technical
        );
    }

    #[test]
    fn keyword_wins_over_default_type() {
        assert_eq!(
            classify("Tell me about a conflict you resolved"),
            QuestionType::Star
        );
    }

    #[test]
    fn multiple_keywords_highest_score_wins() {
        // Has both behavioral (equipo) and architecture (diseño) keywords
        // behavioral_score=1, architecture_score=1 → should prefer architecture
        // since it's listed second... actually both have 1, so it picks based on order.
        assert_eq!(
            classify("¿Cómo diseñaste la solución con tu equipo?"),
            QuestionType::Architecture
        );
    }

    // -----------------------------------------------------------------------
    // Zero-score fallback — no keyword lists matched, heuristic inference
    // -----------------------------------------------------------------------

    #[test]
    fn zero_score_fallback_technical_herramienta() {
        // "qué herramienta" matches the fallback heuristic (line 159)
        // but is NOT in any keyword list → hits line 164
        assert_eq!(
            classify("¿Qué herramienta usaste?"),
            QuestionType::Technical
        );
    }

    #[test]
    fn zero_score_fallback_technical_what_tool() {
        // "what tool" matches the fallback heuristic (line 160)
        // but is NOT in any keyword list → hits line 164
        assert_eq!(classify("What tool did you use?"), QuestionType::Technical);
    }

    #[test]
    fn zero_score_fallback_behavioral_cuando() {
        // "cuándo" is NOT in STAR_KEYWORDS, but matches fallback (line 167) → hits line 174
        assert_eq!(classify("¿Cuándo fue eso?"), QuestionType::Star);
    }

    #[test]
    fn zero_score_fallback_behavioral_when() {
        // "when" (standalone) is NOT in STAR_KEYWORDS, matches fallback (line 168) → hits line 174
        assert_eq!(classify("When did that happen?"), QuestionType::Star);
    }

    #[test]
    fn zero_score_fallback_behavioral_tell_about() {
        // "tell me about your background" -> imperative trigger "tell me",
        // no keyword matches, fallback "tell && about" (line 170-171) → hits line 174
        assert_eq!(
            classify("Tell me about your background"),
            QuestionType::Star
        );
    }

    // -----------------------------------------------------------------------
    // Experience question override
    // -----------------------------------------------------------------------

    #[test]
    fn experience_override_hablame() {
        // "háblame" is NOT in STAR_KEYWORDS but triggers the experience
        // question override (line 185, 188). No other keywords → behavioral_score = 1.
        assert_eq!(classify("Háblame de ti"), QuestionType::Star);
    }

    #[test]
    fn walk_me_through_code_suppresses_experience_override() {
        // "walk me through code": imperative trigger fires, "code" keyword gives
        // technical_score=1. The experience override is suppressed at line 183
        // because the text contains "code". Returns Technical.
        assert_eq!(classify("Walk me through code"), QuestionType::Technical);
    }

    // -----------------------------------------------------------------------
    // Exclusion list priority over imperative triggers
    // -----------------------------------------------------------------------

    #[test]
    fn exclusion_wins_over_imperative_triggers() {
        // "how are you" is in EXCLUSION_LIST and checked before imperative
        // detection → returns None regardless of other content
        assert_eq!(classify("Tell me how are you"), QuestionType::None);
    }

    #[test]
    fn exclusion_wins_with_question_mark() {
        // Even with a question mark, exclusion takes priority
        assert_eq!(
            classify("How are you? Tell me about your team"),
            QuestionType::None
        );
    }

    // -----------------------------------------------------------------------
    // Type tie-breaking priority: Trap > Architecture > Star > Technical
    // -----------------------------------------------------------------------

    #[test]
    fn trap_wins_tie_breaking_priority() {
        // "debilidad" → trap_score=1, "equipo" → behavioral_score=1.
        // Tie: Trap checked first at line 207 → Trap wins.
        assert_eq!(
            classify("¿Cuál es tu mayor debilidad y cómo trabajas en equipo?"),
            QuestionType::Trap
        );
    }

    #[test]
    fn architecture_wins_over_technical_tie() {
        // "diseñaste" → architecture_score=1, "algoritmo" → technical_score=1.
        // Tie: Architecture checked before Star/Technical at line 210 → Architecture wins.
        assert_eq!(
            classify("¿Cómo diseñaste el algoritmo?"),
            QuestionType::Architecture
        );
    }

    #[test]
    fn architecture_wins_over_behavioral_tie() {
        // "escalabilidad" → architecture_score=1, "equipo" → behavioral_score=1.
        // Tie: Architecture checked before behavioral at line 210 → Architecture wins.
        assert_eq!(
            classify("¿Cómo diseñaste la escalabilidad del equipo?"),
            QuestionType::Architecture
        );
    }

    // -----------------------------------------------------------------------
    // Additional imperative trigger coverage
    // -----------------------------------------------------------------------

    #[test]
    fn imperative_trigger_caminenme_por() {
        assert_eq!(
            classify("Camínenme por su experiencia con el equipo"),
            QuestionType::Star
        );
    }

    #[test]
    fn imperative_trigger_platicame() {
        assert_eq!(
            classify("Platícame de un proyecto que lideraste"),
            QuestionType::Star
        );
    }

    #[test]
    fn imperative_trigger_narrame() {
        // "nárrame" triggers imperative detection; "cómo" in fallback gives
        // technical_score=1. No behavioral keywords → Technical.
        assert_eq!(
            classify("Nárrame cómo implementaste el módulo"),
            QuestionType::Technical
        );
    }

    // -----------------------------------------------------------------------
    // Sharps: keyword that appears in multiple lists (e.g. "error")
    // -----------------------------------------------------------------------

    #[test]
    fn keyword_error_in_both_technical_and_star_lists() {
        // "error" appears in both TECHNICAL_KEYWORDS and STAR_KEYWORDS.
        // Both scores are incremented. Tie 1-1. Behavioral checked before
        // Technical → Star wins.
        assert_eq!(
            classify("¿Cómo manejaste ese error del equipo?"),
            QuestionType::Star
        );
    }

    // -----------------------------------------------------------------------
    // classify_text Tauri command — all types
    // -----------------------------------------------------------------------

    #[test]
    fn classify_text_technical() {
        assert_eq!(
            classify_text("¿Qué tecnología usaste?".into()),
            QuestionType::Technical
        );
    }

    #[test]
    fn classify_text_star() {
        assert_eq!(
            classify_text("Cuéntame de un conflicto en tu equipo".into()),
            QuestionType::Star
        );
    }

    #[test]
    fn classify_text_architecture() {
        assert_eq!(
            classify_text("¿Cómo diseñaste la arquitectura?".into()),
            QuestionType::Architecture
        );
    }

    #[test]
    fn classify_text_trap() {
        assert_eq!(
            classify_text("¿Cuál es tu mayor debilidad?".into()),
            QuestionType::Trap
        );
    }

    #[test]
    fn classify_text_none() {
        assert_eq!(
            classify_text("Me gusta programar".into()),
            QuestionType::None
        );
    }

    #[test]
    fn classify_text_small_talk() {
        assert_eq!(classify_text("¿Cómo estás?".into()), QuestionType::None);
    }

    #[test]
    fn classify_text_empty() {
        assert_eq!(classify_text("".into()), QuestionType::None);
    }

    // -----------------------------------------------------------------------
    // type -> string conversion
    // -----------------------------------------------------------------------

    #[test]
    fn question_type_as_str() {
        assert_eq!(QuestionType::Technical.as_str(), "technical");
        assert_eq!(QuestionType::Star.as_str(), "star");
        assert_eq!(QuestionType::Architecture.as_str(), "architecture");
        assert_eq!(QuestionType::Trap.as_str(), "trap");
        assert_eq!(QuestionType::None.as_str(), "none");
    }

    // -----------------------------------------------------------------------
    // Additional edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn classify_empty_after_trim_returns_none() {
        // classify() trims the input first
        assert_eq!(classify("  \t  \n  "), QuestionType::None);
    }

    #[test]
    fn classify_question_mark_only() {
        // Just a "?" — no keywords, imperative triggers, or actual text
        // has_question_mark = true, but everything else is empty → Technical
        assert_eq!(classify("?"), QuestionType::Technical);
    }

    #[test]
    fn classify_question_mark_with_whitespace() {
        assert_eq!(classify("  ?  "), QuestionType::Technical);
    }

    #[test]
    fn classify_uppercase_keywords_still_match() {
        // All-caps keywords should match after to_lowercase()
        assert_eq!(
            classify("¿CÓMO IMPLEMENTASTE LA CACHÉ?"),
            QuestionType::Technical
        );
    }

    #[test]
    fn classify_mixed_case_keywords_match() {
        assert_eq!(
            classify("Tell Me About A Conflict In Your Team"),
            QuestionType::Star
        );
    }

    #[test]
    fn classify_keyword_in_middle_of_text_detected() {
        // "cuándo" is a fallback trigger — it should be detected even
        // when not at the start
        assert_eq!(
            classify("¿Puedes decirme cuándo fue eso?"),
            QuestionType::Star
        );
    }

    #[test]
    fn classify_imperative_not_at_start_but_marked() {
        // "dime" in the body (after leading word) is handled by body check
        assert_eq!(classify("Y dime sobre tu experiencia"), QuestionType::Star);
    }

    #[test]
    fn classify_only_question_mark_no_text_technical_default() {
        // No keywords matched but has question mark → defaults to Technical
        assert_eq!(classify("?"), QuestionType::Technical);
    }

    #[test]
    fn classify_fallback_technical_via_como() {
        // "cómo" in the text triggers the fallback technical_score increment
        assert_eq!(classify("¿Cómo lo hiciste?"), QuestionType::Technical);
    }

    #[test]
    fn classify_fallback_technical_via_what_technology() {
        assert_eq!(
            classify("What technology is best for this?"),
            QuestionType::Technical
        );
    }

    #[test]
    fn classify_fallback_behavioral_via_tell_about() {
        assert_eq!(
            classify("Tell me about your experience"),
            QuestionType::Star
        );
    }

    #[test]
    fn classify_very_long_text_does_not_panic() {
        let long_text = "¿Cómo implementaste ".repeat(100) + "?";
        let result = classify(&long_text);
        // Should not panic and should return a valid QuestionType
        assert!(matches!(
            result,
            QuestionType::Technical | QuestionType::None
        ));
    }

    #[test]
    fn classify_tie_breaking_trap_wins_over_architecture_when_equal() {
        // Trap = 1, Architecture = 1 → Trap wins (priority check)
        // "debilidad" (trap) + "diseño" (architecture), same score
        let text = "¿Cuál es tu mayor debilidad en el diseño?";
        assert_eq!(classify(text), QuestionType::Trap);
    }

    #[test]
    fn classify_architecture_beats_trap_when_score_higher() {
        // Architecture = 2, Trap = 1 → Architecture wins (higher score)
        let text = "¿Cuál es tu mayor debilidad en el diseño de la arquitectura?";
        assert_eq!(classify(text), QuestionType::Architecture);
    }

    #[test]
    fn classify_tie_breaking_architecture_vs_star() {
        // Architecture = 1, Star = 1 → Architecture wins
        let text = "¿Cómo diseñaste la solución trabajando en equipo?";
        assert_eq!(classify(text), QuestionType::Architecture);
    }

    #[test]
    fn classify_tie_breaking_star_vs_technical() {
        // Star = 1, Technical = 1 → Star wins
        let text = "¿Cuéntame de un error que tuviste en el equipo?";
        assert_eq!(classify(text), QuestionType::Star);
    }

    #[test]
    fn classify_tie_breaking_trap_wins_with_two_keywords() {
        // Trap = 2 (debilidad, defecto), Star = 1, Architecture = 0
        // Trap wins because trap_score == max_score
        let text = "¿Cuál es tu mayor debilidad y qué defecto tienes?";
        assert_eq!(classify(text), QuestionType::Trap);
    }

    #[test]
    fn classify_single_trap_keyword_wins_over_technical() {
        // Trap = 1, Technical = 0 → Trap wins
        let text = "¿Cuál es tu mayor debilidad?";
        assert_eq!(classify(text), QuestionType::Trap);
    }

    // -----------------------------------------------------------------------
    // Integration: classify_text Tauri command — all edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn classify_text_whitespace_only() {
        assert_eq!(classify_text("   ".into()), QuestionType::None);
    }

    #[test]
    fn classify_text_special_characters() {
        assert_eq!(
            classify_text("¿Qué librería usaste para async?".into()),
            QuestionType::Technical
        );
    }

    #[test]
    fn classify_text_single_word_imperative() {
        assert_eq!(classify_text("Cuéntame".into()), QuestionType::Star);
    }

    #[test]
    fn classify_text_imperative_with_filler_prefix() {
        assert_eq!(
            classify_text("Bueno cuéntame de un proyecto difícil".into()),
            QuestionType::Star
        );
    }

    // -----------------------------------------------------------------------
    // QuestionType Clone, Copy, PartialEq
    // -----------------------------------------------------------------------

    #[test]
    fn question_type_is_copy() {
        let a = QuestionType::Technical;
        let b = a; // Copy, not move
        assert_eq!(a, b);
    }

    #[test]
    fn question_type_debug_format() {
        let debug = format!("{:?}", QuestionType::Star);
        assert_eq!(debug, "Star");
    }

    // -----------------------------------------------------------------------
    // QuestionType serialization
    // -----------------------------------------------------------------------

    #[test]
    fn question_type_serialize_technical() {
        let json = serde_json::to_string(&QuestionType::Technical).unwrap();
        assert_eq!(json, r#""technical""#);
    }

    #[test]
    fn question_type_serialize_star() {
        let json = serde_json::to_string(&QuestionType::Star).unwrap();
        assert_eq!(json, r#""star""#);
    }

    #[test]
    fn question_type_serialize_none() {
        let json = serde_json::to_string(&QuestionType::None).unwrap();
        assert_eq!(json, r#""none""#);
    }

    #[test]
    fn question_type_deserialize() {
        let q: QuestionType = serde_json::from_str(r#""architecture""#).unwrap();
        assert_eq!(q, QuestionType::Architecture);

        let q: QuestionType = serde_json::from_str(r#""trap""#).unwrap();
        assert_eq!(q, QuestionType::Trap);
    }
}
