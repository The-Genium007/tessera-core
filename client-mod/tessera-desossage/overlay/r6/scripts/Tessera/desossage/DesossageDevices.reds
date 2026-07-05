module Tessera.Desossage

// Leviers dispositifs monde : voyage rapide (kiosques), vendeurs, distributeurs/interactables.
// STUB : journalise l'intention. Corps réel (FastTravelSystem, interactions devices) à pincer en jeu.

public func Tessera_ApplyFastTravel(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  if e.active {
    return;
  }
  // Helper natif du FastTravelSystem : verrouille le voyage rapide (kiosques/dataterms inactifs).
  // Vérifié contre le script décompilé réel (CDPR-Modding-Documentation/Cyberpunk-Scripts,
  // scripts/core/systems/fastTravelSystem.script:850) — la citation précédente (« .swift »,
  // même défaut que le bug police) était bidon mais le symbole s'avère correct.
  FastTravelSystem.ManageFastTravelLock(false, n"tessera_desossage", game);
  FTLog(s"[Tessera/Desossage] voyage rapide → coupé (ManageFastTravelLock false)");
}

// Recherche (script décompilé officiel) : VendorComponent n'a que des getters de données
// (GetVendorID, GetJunkItemIDs...), aucun toggle actif/inactif. Les vendeurs AMBIANTS tombent
// probablement avec `pedestrians` (CommunitySystem.ChangeDensityModifier, cf. DesossagePopulation)
// s'ils sont spawnés comme PNJ communautaires — à tester en jeu. Les vendeurs NOMMÉS (fixes,
// scénarisés) restent un stub : pas de symbole de désactivation trouvé.
//
// Recherche élargie (2026-07-03, dump RTTI complet) : aucune classe "Shop"/"Store"/"Customer"
// pertinente. Cherché aussi un levier GÉNÉRIQUE (masquer/despawn n'importe quel PNJ, peu importe
// son rôle) qui aurait couvert vendeurs ET clients en magasin d'un coup : `entEntity`/
// `ScriptedPuppet` n'exposent aucune méthode visibilité/despawn/hide/destroy ;
// `SmartDespawnRequest`/`MarkDespawnCandidate` existent (événements internes du jeu, mécanisme de
// nettoyage de PNJ) mais n'ont aucun champ propre exposé — pas de point d'accroche scriptable
// trouvé pour les déclencher nous-mêmes. Piste générique épuisée côté RTTI.
// HYPOTHÈSE (à tester en jeu, pas encore fait) : les PNJ ambiants EN INTÉRIEUR (clients dans un
// magasin) sont probablement, comme le trafic, spawnés via le même mécanisme que `pedestrians`
// (zones `communityArea`/`worldCompiledCommunityAreaNode` vues dans le RTTI), donc potentiellement
// déjà coupés par le levier `pedestrians` existant, sans code supplémentaire. Seuls les vendeurs
// NOMMÉS scénarisés (pas ambiants) resteraient un vrai stub sans solution native trouvée.
public func Tessera_ApplyVendors(game: GameInstance, e: ref<DesossageEntry>) -> Void {
  if e.active {
    return;
  }
  FTLog(s"[Tessera/Desossage] (stub) vendeurs → coupés");
}

public func Tessera_ApplyWorldDevices(game: GameInstance, vending: ref<DesossageEntry>, inter: ref<DesossageEntry>) -> Void {
  if !vending.active {
    // Le vrai coupe-circuit vit dans le @wrapMethod(VendingMachineControllerPS) GetActions
    // ci-dessous (hook partagé, s'applique à toutes les instances). Couvre les distributeurs
    // boissons/nourriture (VendingMachineControllerPS). PAS encore couvert : distributeurs
    // d'armes (WeaponVendingMachineControllerPS) et droppoints (DropPointControllerPS) — classes
    // PS sœurs distinctes, même famille de fix mais pas encore fait.
    FTLog(s"[Tessera/Desossage] distributeurs (boissons/nourriture) → coupés (GetActions)");
  }
  if !inter.active {
    // Stub à nouveau (2026-07-05) — cf. note de désactivation ci-dessous.
    FTLog(s"[Tessera/Desossage] (stub) interactables monde (points d'accès) → coupés");
  }
}

// CASSÉ AU COMPILE (confirmé en jeu, 2026-07-05) : `@wrapMethod(AccessPointControllerPS)
// GetActions(...)` échoue avec `[UNRESOLVED_METHOD] no method with this name exists on the
// target type` (redscript_rCURRENT.log). Root cause identifiée via nativedb : contrairement à
// `VendingMachineControllerPS`/`SecurityTurretControllerPS` (cf. DesossageOrder.reds), qui
// déclarent chacune leur PROPRE override de `GetActions` (visible dans `search.py show
// <Classe>`, sans --deep), `AccessPointControllerPS` n'a AUCUN override propre — `GetActions`
// n'existe que sur `ScriptableDeviceComponentPS` (l'ancêtre commun), jamais réintroduit le long
// de la chaîne AccessPointControllerPS → MasterControllerPS → ScriptableDeviceComponentPS.
// `@wrapMethod` exige une méthode explicitement déclarée sur la classe ciblée, pas juste héritée
// (la réflexion RTTI ne fait pas cette distinction, d'où l'erreur de conception initiale).
// PISTE À TESTER (pas encore fait) : wrapper `ScriptableDeviceComponentPS.GetActions`
// directement — couvrirait AccessPointControllerPS (et toute autre PS de device qui n'override
// pas GetActions elle-même) sans toucher aux classes qui ont leur propre override (répartition
// par dispatch virtuel). À valider en jeu via hot-reload Red Hot Tools avant de relancer le jeu
// à chaque essai.

// Coupe-circuit partagé pour les distributeurs boissons/nourriture (toutes instances, un seul
// hook). Signature moderne vérifiée contre un mod publié réel qui wrap la même classe PS
// (rfuzzo/cyberpunk-nexus-script-dump, mod 6927 « Enhanced Vending Machines »,
// _SharedDependencies.reds:380 — `@wrapMethod(VendingMachineControllerPS) ... GetQuickHackActions
// (out outActions:array<ref<DeviceAction>>, context:GetActionsContext)`) ; la méthode elle-même
// (GetActions, pas GetQuickHackActions) et son rôle sont confirmés par le script décompilé
// officiel (CDPR-Modding-Documentation/Cyberpunk-Scripts, vendingMachineController.script:108).
// Lu via DesossageConfig.Default() direct (même raison que le hook police : pas de dépendance à
// l'ordre d'attache des ScriptableSystem).
@wrapMethod(VendingMachineControllerPS)
protected func GetActions(out actions: array<ref<DeviceAction>>, context: GetActionsContext) -> Bool {
  if !DesossageConfig.Default().vendingDevices.active {
    return false;
  }
  return wrappedMethod(actions, context);
}
