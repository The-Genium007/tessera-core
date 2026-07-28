//! Hachages du moteur Cyberpunk, côté serveur. PUR — aucune I/O hors les tests, comme `frame.rs`.
//!
//! # Pourquoi ce module existe
//!
//! Le catalogue de pantins (`puppet_catalog.rs`) est indexé par **noms** (`Character.foo`,
//! `appearance_bar`). Le fil, lui, ne porte que des **hashes** : `AppearanceSpec.base_record`
//! (TweakDBID) et `AppearanceSpec.appearance` (CName), tous deux `ulong`.
//!
//! Rien ne faisait le pont. Conséquence concrète : le serveur **ne pouvait pas valider** ce qu'un
//! client demandait — la case « valider le choix reçu (un client ne doit pas pouvoir demander une
//! paire hors catalogue) » de la roadmap palier 2 était intenable telle quelle, et
//! `puppet_catalog.rs` n'avait aucun consommateur.

/// FNV-1a 64 bits — le hachage de `CName` **et** de `ResourcePath` dans Cyberpunk 2077.
///
/// Vérifié, pas supposé : les tests le confrontent aux **7336 couples (hash, chemin)** de
/// `entity-path-hashes.json`, extraits du jeu et déjà validés d'un autre côté (une extraction
/// WolvenKit `--hash` rend exactement le fichier attendu).
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    // Groupé sur 16 chiffres hexadécimaux, pas 11 : écrit `0x1000_0000_01b3`, le soulignement
    // masque un zéro surnuméraire et la constante devient 0x1000000001b3 — un nombre premier FNV
    // qui n'existe pas. Erreur commise puis attrapée par les vecteurs canoniques ci-dessous.
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// Hash d'un `CName` — l'identité d'une variante d'apparence (`AppearanceSpec.appearance`).
/// **Aucune normalisation** : ni minuscules, ni substitution de séparateur. Le moteur hache la
/// chaîne telle quelle, et une normalisation « de confort » donnerait un hash qui ne correspond à
/// rien (mesuré sur les chemins de ressources : la variante minuscules ne résout aucun fichier).
pub fn cname(name: &str) -> u64 {
    fnv1a64(name.as_bytes())
}

/// Hash d'un `ResourcePath`. Même fonction que `cname`, mais le chemin doit être en forme
/// **antislash** (`base\characters\...`). C'est la seule forme qui résout : la variante avec des
/// barres obliques, comme la variante en minuscules, ne rend aucun fichier (vérifié le 2026-07-27
/// en extrayant par `--hash`). Les chemins sont stockés avec des barres obliques un peu partout —
/// d'où cette conversion faite ici, une fois, plutôt qu'oubliée chez chaque appelant.
pub fn resource_path(path: &str) -> u64 {
    fnv1a64(path.replace('/', "\\").as_bytes())
}

// ─────────────────────────────────────────────────────────────────────────────
// ⛔ TweakDBID (`AppearanceSpec.base_record`) — DÉLIBÉRÉMENT PAS IMPLÉMENTÉ ICI.
//
// L'algorithme est documenté par la communauté (CRC32 du nom sur les 32 bits bas, longueur du nom
// sur les 8 bits suivants), mais **le dépôt ne contient aucun vecteur de vérité** pour le
// confronter : le dump TweakDB (`tools/tweakdb/out/cache.pkl`) indexe les records par NOM, pas par
// id numérique, et `char_hashes` du harnais rend le hash du `entityTemplatePath` (un ResourcePath),
// pas l'id du record.
//
// L'écrire sans pouvoir le vérifier, c'est fabriquer un validateur qui rejetterait des choix
// légitimes — un mur silencieux à la création de personnage, exactement la classe de bug que le
// protocole de sondage interdit. La règle du dépôt est explicite : ne jamais deviner, mesurer.
//
// Ce qu'il faut, et c'est une poignée de secondes en jeu : `TweakDBID.new("Character.<X>")` depuis
// CET rend l'id complet. L'action `tdb_hash` du harnais produit ces couples pour une liste de
// records ; une fois le fichier de vecteurs en main, cette fonction s'écrit et se vérifie d'un
// coup, et la validation de `base_record` se branche derrière.
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Vecteurs canoniques publics de FNV-1a 64 bits — indépendants de Cyberpunk. Ils isolent une
    /// erreur d'algorithme d'une erreur de forme d'entrée (séparateur, casse) : si ceux-ci passent
    /// et que les chemins du jeu échouent, le fautif est la normalisation, pas le hachage.
    #[test]
    fn fnv1a64_matches_the_canonical_reference_vectors() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
    }

    fn hashes_json() -> serde_json::Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("entity-path-hashes.json");
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("lecture de {} : {e}", path.display()));
        serde_json::from_str(&raw).expect("entity-path-hashes.json doit rester du JSON valide")
    }

    /// La vraie preuve : les 7336 couples extraits du jeu doivent TOUS retomber sur leur hash.
    /// Un echantillon ne suffirait pas — c'est precisement sur les cas tordus (accents, majuscules,
    /// chemins ep1) qu'une normalisation abusive se trahit.
    #[test]
    fn resource_path_reproduces_every_hash_extracted_from_the_game() {
        let json = hashes_json();
        let table = json["hash_to_path"]
            .as_object()
            .expect("hash_to_path doit etre un objet");
        assert!(
            table.len() > 7000,
            "corpus etonnamment petit ({}) — le fichier a-t-il ete tronque ?",
            table.len()
        );
        let mut mismatches: Vec<String> = Vec::new();
        for (hash_str, path) in table {
            let expected: u64 = hash_str.parse().expect("les cles sont des u64 decimaux");
            let path = path.as_str().expect("les valeurs sont des chemins");
            let got = resource_path(path);
            if got != expected {
                mismatches.push(format!("{path} : attendu {expected}, obtenu {got}"));
            }
        }
        assert!(
            mismatches.is_empty(),
            "{} chemin(s) sur {} ne retombent pas sur leur hash :\n{}",
            mismatches.len(),
            table.len(),
            mismatches
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    /// Le contre-test qui donne son sens au precedent : la variante « de confort » (barres obliques
    /// + minuscules) NE doit PAS retomber sur les memes hashes. Sans lui, un test vert ne
    /// distinguerait pas « la forme est la bonne » de « la forme n'a aucune importance ».
    #[test]
    fn the_lowercased_slash_variant_does_not_reproduce_the_hashes() {
        let json = hashes_json();
        let table = json["hash_to_path"]
            .as_object()
            .expect("hash_to_path doit etre un objet");
        let agreeing = table
            .iter()
            .filter(|(hash_str, path)| {
                let expected: u64 = hash_str.parse().unwrap();
                fnv1a64(path.as_str().unwrap().to_lowercase().as_bytes()) == expected
            })
            .count();
        assert_eq!(
            agreeing, 0,
            "la variante minuscules/barres obliques ne doit retomber sur AUCUN hash"
        );
    }

    #[test]
    fn cname_does_not_normalise_its_input() {
        // Deux noms qui ne different que par la casse doivent donner deux hashes differents —
        // c'est ce qui interdit d'« arranger » un nom d'apparence avant de le hacher.
        assert_ne!(cname("Appearance_Foo"), cname("appearance_foo"));
    }
}
