//! Migration ponctuelle : chaque `player_records` existant devient le personnage UNIQUE de ce
//! compte (mapping direct subject -> 1 character_id), pseudonyme initial = ancien display_name
//! (garanti sans collision par l'ancienne contrainte unique déjà en place sur player_records).
//! Idempotente : une exécution répétée ne duplique jamais un personnage déjà migré (vérifie
//! l'existence par owner avant de créer).

use crate::character_store::CharacterStore;
use sqlx::PgPool;

/// Migre tous les `player_records` n'ayant PAS encore de personnage correspondant. Renvoie le
/// nombre de personnages créés par cet appel (0 si tout était déjà migré).
#[allow(clippy::type_complexity)]
pub async fn migrate_player_records_to_characters(pool: &PgPool) -> Result<usize, sqlx::Error> {
    let records: Vec<(String, String, Vec<f32>, Option<Vec<f32>>)> = sqlx::query_as(
        "SELECT subject, display_name, last_position, residence FROM player_records",
    )
    .fetch_all(pool)
    .await?;

    let mut migrated = 0;
    let mut store = CharacterStore::new(pool.clone());
    for (subject, display_name, last_position, residence) in records {
        let existing = store
            .list_by_owner_async(&subject)
            .await
            .unwrap_or_default();
        if !existing.is_empty() {
            continue; // déjà migré
        }
        // create_async initialise last_position à [0,0,0] — on écrase ensuite avec la vraie
        // position connue via save_progress_async pour ne pas dupliquer la logique SQL ici.
        match store.create_async(&subject, &display_name, b"").await {
            Ok(character) => {
                let pos = [last_position[0], last_position[1], last_position[2]];
                let _ = store.save_progress_async(character.id, pos).await;
                let _ = residence; // résidence non reportée dans ce plan (champ optionnel, cf. hors périmètre)
                migrated += 1;
            }
            Err(_) => continue, // pseudonyme déjà pris par un AUTRE compte -- ne devrait jamais
                                // arriver (contrainte unique display_name déjà en place avant
                                // cette migration), mais on ne panique pas sur une donnée réelle.
        }
    }
    Ok(migrated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[sqlx::test(migrations = "./migrations")]
    async fn migrates_a_player_record_into_exactly_one_character(pool: PgPool) {
        sqlx::query(
            "INSERT INTO player_records (subject, display_name, last_position)
             VALUES ($1, $2, $3)",
        )
        .bind("sub-1")
        .bind("Lucas")
        .bind(&[1.0f32, 2.0, 3.0][..])
        .execute(&pool)
        .await
        .unwrap();

        let migrated = migrate_player_records_to_characters(&pool).await.unwrap();
        assert_eq!(migrated, 1);

        let store = CharacterStore::new(pool.clone());
        let characters = store.list_by_owner_async("sub-1").await.unwrap();
        assert_eq!(characters.len(), 1);
        assert_eq!(characters[0].pseudonym, "Lucas");
        assert_eq!(characters[0].last_position, [1.0, 2.0, 3.0]);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn running_migration_twice_does_not_duplicate_characters(pool: PgPool) {
        sqlx::query(
            "INSERT INTO player_records (subject, display_name, last_position)
             VALUES ($1, $2, $3)",
        )
        .bind("sub-1")
        .bind("Lucas")
        .bind(&[0.0f32, 0.0, 0.0][..])
        .execute(&pool)
        .await
        .unwrap();

        migrate_player_records_to_characters(&pool).await.unwrap();
        let second_run = migrate_player_records_to_characters(&pool).await.unwrap();
        assert_eq!(
            second_run, 0,
            "la deuxième exécution ne doit rien migrer de plus"
        );

        let store = CharacterStore::new(pool.clone());
        let characters = store.list_by_owner_async("sub-1").await.unwrap();
        assert_eq!(
            characters.len(),
            1,
            "toujours un seul personnage après 2 exécutions"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn migrates_multiple_distinct_accounts_independently(pool: PgPool) {
        for (subject, name) in [("sub-1", "Alice"), ("sub-2", "Bob")] {
            sqlx::query(
                "INSERT INTO player_records (subject, display_name, last_position) VALUES ($1, $2, $3)",
            )
            .bind(subject)
            .bind(name)
            .bind(&[0.0f32, 0.0, 0.0][..])
            .execute(&pool)
            .await
            .unwrap();
        }
        let migrated = migrate_player_records_to_characters(&pool).await.unwrap();
        assert_eq!(migrated, 2);
    }
}
