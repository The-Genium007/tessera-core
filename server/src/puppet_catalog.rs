//! Catalogue des avatars proposables au joueur (modèle PRESET, pivot du 2026-07-27).
//!
//! Suit le patron de `elevator_catalog.rs` : struct `Deserialize` + `parse_and_validate` **pur**
//! séparé de `load` (I/O), et un enum d'erreur dédié.
//!
//! # À quoi ça sert
//!
//! Le chantier « avatar joueur haute-fidélité » (B1/B2) est en pause : le rendu d'un corps-joueur
//! au visage exact bute sur un verrou natif non résolu. Le multi utilise donc des **presets** — le
//! joueur choisit un avatar dans un catalogue, et le fil ne porte que deux hashes
//! (`base_record` + `appearance`, cf. `appearance_relay`).
//!
//! Ce module tient le catalogue et répond à **deux** questions :
//! 1. que proposer au joueur (par catégorie RP : `corpo` / `gosse_des_rues` / `nomad`) ;
//! 2. **un choix reçu d'un client est-il légitime ?** — c'est la raison d'être principale.
//!
//! # Pourquoi la validation n'est pas optionnelle
//!
//! Même règle que la décision n°2 du catalogue d'ascenseurs : **la liste vient d'ICI, jamais d'une
//! donnée client**. Sans ce contrôle, un client modifié choisirait n'importe quel couple
//! `(record, apparence)` — donc n'importe quelle entité du jeu comme avatar, y compris des PNJ
//! scénarisés ou des entités qui ne rendent rien du tout (mesuré : un corps-joueur spawné
//! standalone est invisible). Le serveur relaierait ensuite ce choix à tous les autres joueurs.
//!
//! # Provenance des données (`puppet-catalog.json`)
//!
//! Extrait des archives du jeu 2.31, **jeu de base ET ep1 fusionnés** — Phantom Liberty remplace
//! les entités de foule, et c'est vers elles que pointent les records ; sans ep1, la résolution des
//! records de référence tombe à 0/11. Le nom d'apparence utilisable est celui **déclaré par le
//! `.ent`** (`RootChunk.appearances[].name`), pas celui du `.app` : c'est une table de
//! correspondance, pas une formule dérivable. Un `.ent` n'expose qu'un sous-ensemble de son `.app`.
//! Détail : `research/FINDINGS.md` (Q3) et l'en-tête du JSON.

use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fmt;

/// Une catégorie RP. Volontairement une simple `String` : les catégories vivent dans la donnée,
/// pas dans le code — en ajouter une ne doit pas demander un recompile du serveur.
pub type Category = String;

#[derive(Debug, Deserialize)]
struct RawCatalog {
    archetypes: HashMap<String, RawArchetype>,
}

#[derive(Debug, Deserialize)]
struct RawArchetype {
    category: String,
    #[serde(default)]
    records: Vec<String>,
    #[serde(default)]
    appearances: Vec<String>,
}

/// Un avatar proposable : un `base_record` et l'apparence épinglée qui va avec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PuppetChoice {
    pub record: String,
    pub appearance: String,
    pub category: Category,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CatalogError {
    Json(String),
    /// Un archétype sans record OU sans apparence ne peut produire aucun choix : c'est une entrée
    /// morte, et la laisser passer ferait mentir les compteurs servis au client.
    EmptyArchetype(String),
    /// Catalogue sans aucun choix exploitable — presque sûrement un mauvais fichier.
    NoChoices,
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CatalogError::Json(e) => write!(f, "catalogue illisible : {e}"),
            CatalogError::EmptyArchetype(a) => {
                write!(f, "archétype « {a} » sans record ou sans apparence")
            }
            CatalogError::NoChoices => write!(f, "catalogue sans aucun choix exploitable"),
        }
    }
}

#[derive(Debug)]
pub struct PuppetCatalog {
    /// record → apparences autorisées. C'est l'index de la validation, donc un `HashSet` :
    /// le contrôle est sur le chemin d'un message client, il doit être en O(1).
    allowed: HashMap<String, HashSet<String>>,
    /// record → catégorie.
    category_of: HashMap<String, Category>,
}

impl PuppetCatalog {
    /// PUR : aucune I/O. Sépare le parsing du chargement pour rester testable sans fichier.
    pub fn parse_and_validate(json: &str) -> Result<Self, CatalogError> {
        let raw: RawCatalog =
            serde_json::from_str(json).map_err(|e| CatalogError::Json(e.to_string()))?;

        let mut allowed: HashMap<String, HashSet<String>> = HashMap::new();
        let mut category_of: HashMap<String, Category> = HashMap::new();

        for (name, a) in raw.archetypes {
            if a.records.is_empty() || a.appearances.is_empty() {
                return Err(CatalogError::EmptyArchetype(name));
            }
            for rec in &a.records {
                // Un même record peut apparaître dans plusieurs archétypes : on UNIT les
                // apparences plutôt que d'écraser, sinon un choix légitime serait refusé selon
                // l'ordre de lecture d'une HashMap — donc de façon non déterministe.
                let set = allowed.entry(rec.clone()).or_default();
                set.extend(a.appearances.iter().cloned());
                category_of
                    .entry(rec.clone())
                    .or_insert_with(|| a.category.clone());
            }
        }

        if allowed.is_empty() {
            return Err(CatalogError::NoChoices);
        }
        Ok(Self {
            allowed,
            category_of,
        })
    }

    pub fn load(path: &std::path::Path) -> Result<Self, CatalogError> {
        let txt = std::fs::read_to_string(path)
            .map_err(|e| CatalogError::Json(format!("{}: {e}", path.display())))?;
        Self::parse_and_validate(&txt)
    }

    /// **Le contrôle de sécurité.** Un couple reçu d'un client n'est accepté que s'il figure au
    /// catalogue. Tout le reste — record inconnu, apparence d'un autre archétype, chaîne vide —
    /// est refusé.
    pub fn is_valid_choice(&self, record: &str, appearance: &str) -> bool {
        self.allowed
            .get(record)
            .is_some_and(|set| set.contains(appearance))
    }

    pub fn category_of(&self, record: &str) -> Option<&str> {
        self.category_of.get(record).map(|s| s.as_str())
    }

    pub fn categories(&self) -> Vec<Category> {
        let mut v: Vec<Category> = self
            .category_of
            .values()
            .cloned()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        v.sort();
        v
    }

    /// Tous les choix d'une catégorie, triés — l'ordre servi au client doit être **stable** d'un
    /// démarrage à l'autre (une `HashMap` ne l'est pas), sinon la liste bouge sous les yeux du
    /// joueur entre deux sessions.
    pub fn choices_in(&self, category: &str) -> Vec<PuppetChoice> {
        let mut out = Vec::new();
        for (rec, apps) in &self.allowed {
            if self.category_of.get(rec).map(|c| c.as_str()) != Some(category) {
                continue;
            }
            for app in apps {
                out.push(PuppetChoice {
                    record: rec.clone(),
                    appearance: app.clone(),
                    category: category.to_string(),
                });
            }
        }
        out.sort_by(|a, b| (&a.record, &a.appearance).cmp(&(&b.record, &b.appearance)));
        out
    }

    pub fn record_count(&self) -> usize {
        self.allowed.len()
    }

    pub fn choice_count(&self) -> usize {
        self.allowed.values().map(|s| s.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "_meta": { "note": "les champs inconnus doivent etre ignores" },
      "archetypes": {
        "citizen__ep1_rich_wa": {
          "category": "corpo",
          "records": ["Character.CitizenRichFemale", "Character.CitizenRichFemaleCasual"],
          "appearances": ["citizen__rich_wa_rich_17", "citizen__rich_wa_rich_23_casual"]
        },
        "citizen__ep1_biker_ma": {
          "category": "nomad",
          "records": ["Character.CitizenBikerMale"],
          "appearances": ["citizen__biker_ma_biker_01"]
        }
      }
    }"#;

    fn cat() -> PuppetCatalog {
        PuppetCatalog::parse_and_validate(SAMPLE).expect("le catalogue d'exemple doit parser")
    }

    #[test]
    fn parse_ignore_les_champs_inconnus() {
        // Le vrai fichier porte `_meta` et `totaux` : le parsing ne doit pas s'en offusquer,
        // sinon toute annotation documentaire casserait le serveur.
        let c = cat();
        assert_eq!(c.record_count(), 3);
        assert_eq!(c.choice_count(), 5);
    }

    #[test]
    fn accepte_un_couple_du_catalogue() {
        let c = cat();
        assert!(c.is_valid_choice("Character.CitizenRichFemale", "citizen__rich_wa_rich_17"));
        assert!(c.is_valid_choice("Character.CitizenBikerMale", "citizen__biker_ma_biker_01"));
    }

    #[test]
    fn refuse_une_apparence_d_un_autre_archetype() {
        // LE cas d'attaque : le record existe, l'apparence existe — mais pas ensemble. Un client
        // modifie ne doit pas pouvoir recomposer une paire arbitraire.
        let c = cat();
        assert!(!c.is_valid_choice("Character.CitizenBikerMale", "citizen__rich_wa_rich_17"));
    }

    #[test]
    fn refuse_record_inconnu_et_chaines_vides() {
        let c = cat();
        assert!(!c.is_valid_choice("Character.AdamSmasher", "citizen__rich_wa_rich_17"));
        assert!(!c.is_valid_choice("", ""));
        assert!(!c.is_valid_choice("Character.CitizenRichFemale", ""));
    }

    #[test]
    fn refuse_un_archetype_vide() {
        // Une entree sans record ou sans apparence ne produit aucun choix : la laisser passer
        // ferait mentir les compteurs servis au client.
        let json =
            r#"{"archetypes":{"vide":{"category":"corpo","records":[],"appearances":["a"]}}}"#;
        // `unwrap_err` plutot que `assert_eq!` sur le Result : comparer le Result entier
        // exigerait `PartialEq` sur le catalogue, ce qui n'a aucun sens metier.
        assert_eq!(
            PuppetCatalog::parse_and_validate(json).unwrap_err(),
            CatalogError::EmptyArchetype("vide".to_string())
        );
    }

    #[test]
    fn un_record_partage_par_deux_archetypes_cumule_ses_apparences() {
        // Sinon l'acceptation dependrait de l'ordre de lecture d'une HashMap — donc du hasard.
        let json = r#"{"archetypes":{
          "a":{"category":"corpo","records":["R"],"appearances":["x"]},
          "b":{"category":"corpo","records":["R"],"appearances":["y"]}}}"#;
        let c = PuppetCatalog::parse_and_validate(json).unwrap();
        assert!(c.is_valid_choice("R", "x"));
        assert!(c.is_valid_choice("R", "y"));
    }

    #[test]
    fn les_choix_sont_tries_donc_stables_entre_deux_demarrages() {
        let c = cat();
        let a = c.choices_in("corpo");
        let b = c.choices_in("corpo");
        assert_eq!(a, b);
        assert!(a
            .windows(2)
            .all(|w| (&w[0].record, &w[0].appearance) <= (&w[1].record, &w[1].appearance)));
        assert_eq!(a.len(), 4);
    }

    #[test]
    fn categories_triees_et_dedupliquees() {
        assert_eq!(
            cat().categories(),
            vec!["corpo".to_string(), "nomad".to_string()]
        );
    }

    #[test]
    fn json_invalide_rend_une_erreur_lisible() {
        let e = PuppetCatalog::parse_and_validate("{pas du json").unwrap_err();
        assert!(matches!(e, CatalogError::Json(_)));
        assert!(e.to_string().contains("catalogue illisible"));
    }

    #[test]
    fn le_vrai_catalogue_du_depot_parse_et_valide_les_couples_verifies_en_jeu() {
        // Test d'intégration sur la donnée réelle : si l'extraction change, ce test tombe.
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("puppet-catalog.json");
        let c = PuppetCatalog::load(&p).expect("le catalogue du dépôt doit parser");
        assert!(c.choice_count() > 1000, "catalogue anormalement petit");
        // Deux des 11 couples vérifiés en jeu (2026-07-25/27).
        assert!(c.is_valid_choice("Character.CitizenBikerMale", "citizen__biker_ma_biker_01"));
        assert!(c.is_valid_choice(
            "Character.AsianMale",
            "king_of_the_stoop_ma_king_of_the_stoop_ma_04"
        ));
        // Et un couple recomposé, qui doit être refusé.
        assert!(!c.is_valid_choice(
            "Character.CitizenBikerMale",
            "king_of_the_stoop_ma_king_of_the_stoop_ma_04"
        ));
    }
}
