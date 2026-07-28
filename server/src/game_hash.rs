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

/// CRC-32 IEEE (polynôme inversé `0xEDB88320`, init et XOR final à `0xFFFF_FFFF`) — la brique du
/// `TweakDBID`. Implémenté sans table : le corpus à hacher tient en quelques centaines de noms au
/// démarrage, la table de 1 Kio ne paierait pas sa complexité.
fn crc32_ieee(bytes: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg(); // 0xFFFFFFFF si bit bas à 1, sinon 0
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Hash d'un `TweakDBID` — l'identité d'un record TweakDB (`AppearanceSpec.base_record`).
/// **CRC-32 IEEE du nom sur les 32 bits bas, longueur du nom sur les 8 bits suivants.**
///
/// # Vérifié sans lancer le jeu
///
/// `r6/cache/tweakdb.bin` et `tweakdb_ep1.bin` ne contiennent pas les noms — c'est tout l'intérêt
/// d'un hash — mais ils contiennent **l'ensemble des ids valides**. Il suffit donc de calculer l'id
/// d'un nom connu et d'en chercher les octets dans le binaire. Mesure du 2026-07-28
/// (`tools/tweakdb/verify_ids.py`) :
///
/// | échantillon | base | ep1 | union |
/// | --- | --- | --- | --- |
/// | tous les `Character.*` (7818) | 5929 | 7818 | **7818/7818** |
/// | 800 records au hasard | 706 | 779 | 779/800 |
/// | **contre-témoin** : 500 noms fabriqués | 0 | 0 | **0/500** |
///
/// Le contre-témoin est ce qui donne son sens au reste : un mauvais nom ne tombe jamais par
/// hasard, donc les correspondances ne sont pas du bruit. Et la variante « CRC32 sans XOR final »
/// donne 0/200 — l'algorithme retenu n'est pas un choix parmi plusieurs qui marcheraient.
///
/// Les ~21 manquants sur 800 sont des records présents dans les **sources texte** mais pas compilés
/// dans le binaire (schémas, abstraits) — pas un échec de hachage.
///
/// La suite Rust, elle, ne lit aucun fichier du jeu : elle rejoue 262 vecteurs figés dans
/// `tweakdb-id-vectors.json`, pour rester verte sur macOS.
pub fn tweakdb_id(name: &str) -> u64 {
    u64::from(crc32_ieee(name.as_bytes())) | ((name.len() as u64) << 32)
}

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

    /// Vecteur canonique universel de CRC-32 IEEE. Meme role que pour FNV : isoler une erreur
    /// d'algorithme d'une erreur de composition (decalage de la longueur, boutisme).
    #[test]
    fn crc32_matches_the_canonical_check_value() {
        assert_eq!(crc32_ieee(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn tweakdb_id_packs_the_length_above_the_crc() {
        let name = "Character.foo";
        let id = tweakdb_id(name);
        assert_eq!(id & 0xFFFF_FFFF, u64::from(crc32_ieee(name.as_bytes())));
        assert_eq!(id >> 32, name.len() as u64);
    }

    /// Deux noms de MEME longueur mais de contenu different, et deux noms de meme contenu tronque
    /// a des longueurs differentes, doivent tous donner des ids distincts. C'est ce qui verrouille
    /// la composition : un id qui n'emporterait que le CRC passerait le premier cas et raterait le
    /// second.
    #[test]
    fn tweakdb_id_separates_both_on_content_and_on_length() {
        assert_ne!(tweakdb_id("Character.aaa"), tweakdb_id("Character.aab"));
        assert_ne!(tweakdb_id("Character.aaa"), tweakdb_id("Character.aaa "));
    }

    /// La vraie preuve, portable : 262 vecteurs figes, chacun retrouve dans les binaires TweakDB du
    /// jeu 2.31 au moment de leur generation (`tools/tweakdb/verify_ids.py`). Ce test ne lit aucun
    /// fichier du jeu, donc il reste vert sur macOS.
    #[test]
    fn tweakdb_id_reproduces_every_frozen_vector() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tweakdb-id-vectors.json");
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("lecture de {} : {e}", path.display()));
        let json: serde_json::Value =
            serde_json::from_str(&raw).expect("tweakdb-id-vectors.json doit rester du JSON valide");
        let table = json["name_to_id"]
            .as_object()
            .expect("name_to_id doit etre un objet");
        assert!(
            table.len() >= 200,
            "corpus de vecteurs etonnamment petit ({}) — fichier tronque ?",
            table.len()
        );
        let mut mismatches: Vec<String> = Vec::new();
        for (name, id_str) in table {
            let expected: u64 = id_str.as_str().unwrap().parse().expect("id u64 decimal");
            let got = tweakdb_id(name);
            if got != expected {
                mismatches.push(format!("{name} : attendu {expected}, obtenu {got}"));
            }
        }
        assert!(
            mismatches.is_empty(),
            "{} vecteur(s) sur {} ne retombent pas :
{}",
            mismatches.len(),
            table.len(),
            mismatches.iter().take(5).cloned().collect::<Vec<_>>().join(
                "
"
            )
        );
    }
}
