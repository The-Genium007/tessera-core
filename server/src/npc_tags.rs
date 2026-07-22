//! Dérivation des tags de curation à partir d'un NOM d'archétype CDPR (spec ambiance §4, pont
//! Phase A→B). `CorpoManCoffee` → `["corpo", "male"]`, `ChildPoorDE` → `["child", ...]`.
//!
//! Pourquoi une dérivation testée et non un dictionnaire écrit à la main : le corpus d'ambiance
//! fait 216 archétypes, et un tag FAUX est silencieux — si `child` n'est pas dérivé, l'exclure
//! ne bloque rien (spec ambiance §5, « le pire mode de défaillance est le silence »). Chaque règle
//! est donc ancrée sur un motif réellement observé dans le pool, et vérifiée par un test nommé.
//!
//! Règle de non-silence : `derive_tags` qui renvoie une liste VIDE est un signal, pas un défaut —
//! c'est un nom qu'aucune règle ne reconnaît (contaminant ou famille neuve). L'outil générateur
//! (`bin/npc_catalog_gen.rs`) DOIT rapporter ces noms bruyamment plutôt que d'émettre un archétype
//! sans tags, qui serait inexcluable.

/// Dérive les tags de curation d'un nom d'archétype CDPR. Déterministe, tags triés et dédupliqués.
/// Liste vide = nom non reconnu (à rapporter, pas à ignorer).
pub fn derive_tags(name: &str) -> Vec<String> {
    let lc = name.to_lowercase();
    let mut tags: Vec<String> = Vec::new();
    let mut add = |t: &str| {
        let t = t.to_string();
        if !tags.contains(&t) {
            tags.push(t);
        }
    };

    // --- GENRE. `female`/`woman` d'abord : "Female" CONTIENT "male", l'ordre inverse mistaggerait
    //     toutes les femmes en hommes. `_wa`/`_ma` sont les suffixes des LightCrowd_<district>_xx.
    if lc.contains("female")
        || lc.contains("woman")
        || lc.contains("queen")
        || lc.ends_with("_wa")
        || lc.contains("_wa_")
    {
        add("female");
    } else if lc.contains("male")
        || lc.contains("man")
        || lc.contains("king")
        || lc.ends_with("_ma")
        || lc.contains("_ma_")
    {
        // `king`/`queen` couvrent StoopKing/StoopQueen, seuls noms du pool sans man/woman/_xx.
        add("male");
    }

    // --- RÔLE / FACTION sensibles : ce sont les clés d'exclusion les plus demandées. Tagués même
    //     si présents "par erreur" dans le pool d'ambiance, précisément pour être exclubles.
    if lc.contains("police") {
        add("police");
    }
    if lc.contains("guard") || lc.contains("security") || lc.contains("bouncer") {
        add("security");
    }
    if lc.contains("corpo") || lc.contains("arasaka") || lc.contains("militech") {
        add("corpo");
    }
    if lc.contains("aldecaldo") || lc.contains("nomad") {
        add("nomad");
    }

    // --- SENSIBLE (contenu). `child` et `sexworker` sont les exclusions typiques d'un serveur RP.
    if lc.contains("child") {
        add("child");
    }
    if lc.contains("sexworker")
        || lc.contains("prostitute")
        || lc.contains("tubedancer")
        || lc.contains("servicedancer")
    {
        add("sexworker");
    }

    // --- STYLE DE VIE / MÉTIER.
    if lc.contains("homeless") || lc.contains("hobo") {
        add("homeless");
    }
    if lc.contains("junkie") || lc.contains("drunk") {
        add("junkie");
    }
    if lc.contains("nightlife") {
        add("nightlife");
    }
    if lc.contains("lowlife") {
        add("lowlife");
    }
    if lc.contains("worker") || lc.contains("municipial") {
        add("worker");
    }
    if lc.contains("vendor") {
        add("vendor");
    }
    if lc.contains("tenant") {
        add("tenant");
    }
    if lc.contains("cook")
        || lc.contains("waitress")
        || lc.contains("service")
        || lc.contains("dining")
        || lc.contains("mox")
    {
        add("service");
    }
    if lc.contains("media") || lc.contains("journalist") {
        add("media");
    }
    if lc.contains("medical") || lc.contains("nurse") {
        add("medical");
    }
    if lc.contains("monk") || lc.contains("religious") || lc.contains("bikhu") {
        add("religious");
    }
    if lc.contains("jock") || lc.contains("workout") {
        add("athletic");
    }
    if lc.contains("freak") {
        add("freak");
    }
    if lc.contains("doll") {
        add("doll");
    }
    if lc.contains("slacker") {
        add("slacker");
    }
    if lc.contains("stoop") {
        // « Stoop King/Queen » : argot de Night City pour qui traîne sur son perron. Oisif.
        add("loiterer");
    }
    if lc.contains("scientist") {
        add("scientist");
    }

    // --- FOULE générique (LightCrowd / MorningCrowd). `morning` en plus pour la pondération future.
    if lc.contains("crowd") {
        add("crowd");
        if lc.contains("morning") {
            add("morning");
        }
    }

    // --- DESCRIPTEURS.
    if lc.contains("rich") {
        add("rich");
    }
    if lc.contains("obese") || lc.contains("fatty") || lc.contains("chubby") {
        add("obese");
    }
    if lc.contains("youngster") || lc.contains("teenager") {
        add("youngster");
    }
    if lc.contains("asian") {
        add("asian");
    }
    if lc.contains("creole") {
        add("creole");
    }
    if lc.contains("nonbinary") {
        add("nonbinary");
    }

    tags.sort();
    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Chaque cas est un nom RÉEL du pool d'ambiance (Phase A) et le(s) tag(s) attendu(s) — pas un
    /// exemple inventé (règle 9 : valider sur des cas réels, dont ceux censés surprendre).
    #[test]
    fn derives_lifestyle_and_gender_from_real_names() {
        assert_eq!(derive_tags("CorpoManCoffee"), vec!["corpo", "male"]);
        assert_eq!(derive_tags("HomelessManDE"), vec!["homeless", "male"]);
        assert_eq!(derive_tags("NightlifeWoman"), vec!["female", "nightlife"]);
        assert_eq!(derive_tags("VendorFemaleDE"), vec!["female", "vendor"]);
    }

    #[test]
    fn female_is_not_mistaken_for_male_despite_containing_the_substring() {
        // Le piège classique : "Female" contient "male". Une femme ne doit JAMAIS être taguée male.
        let t = derive_tags("CorpoWomanArasaka");
        assert!(t.contains(&"female".to_string()));
        assert!(
            !t.contains(&"male".to_string()),
            "CorpoWoman n'est pas male"
        );
    }

    #[test]
    fn sensitive_tags_are_derived_for_exclusion() {
        // Les deux exclusions que Lucas a nommées explicitement doivent tomber juste.
        assert!(derive_tags("ChildPoorDE").contains(&"child".to_string()));
        assert!(derive_tags("ProstituteFemale").contains(&"sexworker".to_string()));
        assert!(derive_tags("SexworkerMaleDoll").contains(&"sexworker".to_string()));
    }

    #[test]
    fn contaminants_get_tagged_so_they_can_be_excluded() {
        // Ces noms ont fui dans le pool d'ambiance de Phase A. Les taguer police/security/corpo
        // permet à l'opérateur de les rejeter (au lieu qu'ils passent pour des passants neutres).
        assert!(derive_tags("Policeman").contains(&"police".to_string()));
        assert!(derive_tags("GuardMaleDE").contains(&"security".to_string()));
        assert!(derive_tags("ArasakaScientist").contains(&"corpo".to_string()));
    }

    #[test]
    fn light_crowd_district_names_get_crowd_and_gender() {
        let t = derive_tags("LightCrowd_kabuki_wa");
        assert!(t.contains(&"crowd".to_string()));
        assert!(t.contains(&"female".to_string()), "_wa => female");
    }

    #[test]
    fn morning_crowd_carries_both_crowd_and_morning() {
        let t = derive_tags("MorningCrowdManCoffee");
        assert!(t.contains(&"crowd".to_string()));
        assert!(t.contains(&"morning".to_string()));
        assert!(t.contains(&"male".to_string()));
    }

    #[test]
    fn tags_are_sorted_and_deduplicated() {
        let t = derive_tags("CorpoManArasaka"); // "corpo" ET "arasaka" -> un seul "corpo"
        let mut sorted = t.clone();
        sorted.sort();
        assert_eq!(t, sorted, "tags triés");
        assert_eq!(
            t.iter().filter(|x| *x == "corpo").count(),
            1,
            "pas de doublon corpo"
        );
    }

    #[test]
    fn stoop_king_and_queen_derive_gender_and_loiterer() {
        // Cas rattrapés après que le générateur les a signalés sur le pool réel (règle de
        // non-silence en action : le signal a produit une règle, pas un tag muet).
        let king = derive_tags("StoopKingBig");
        assert!(king.contains(&"male".to_string()), "King => male");
        assert!(king.contains(&"loiterer".to_string()));
        let queen = derive_tags("StoopQueenNoReaction");
        assert!(queen.contains(&"female".to_string()), "Queen => female");
        assert!(queen.contains(&"loiterer".to_string()));
    }

    #[test]
    fn an_unrecognised_name_returns_empty_which_is_a_signal() {
        // Non-silence : un nom sans aucun motif connu rend une liste VIDE, que le générateur doit
        // rapporter. On ne fabrique jamais un tag par défaut qui masquerait un contaminant.
        assert!(derive_tags("Xyzzy42").is_empty());
    }
}
