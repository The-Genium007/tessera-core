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
    // Couvert par le @wrapMethod(AccessPointControllerPS) GetActions ci-dessous — couvre les
    // points d'accès/hackables ambiants. Les ripperdocs restent hors périmètre (UI de vente
    // scénarisée, pas une PS de device — même famille de recherche que "vendors").
    FTLog(s"[Tessera/Desossage] interactables monde (points d'accès) → gérés par hook AccessPointControllerPS.GetActions");
  }
}

// Même pattern que VendingMachineControllerPS/SecurityTurretControllerPS (cf. DesossageOrder.reds) :
// `GetActions` est déclarée sur `ScriptableDeviceComponentPS`, classe mère commune à toutes les PS
// de devices. `AccessPointControllerPS` couvre les points d'accès/panneaux hackables ambiants —
// confirmé présent dans le dump RTTI (WopsS/RED4ext.NativeDB, classes/AccessPointControllerPS.json).
// Coupe le menu d'interaction/quickhack, pas la présence physique du device.
// PIN IN-GAME : à confirmer (le menu de hack doit disparaître sur les points d'accès ambiants).
@wrapMethod(AccessPointControllerPS)
protected func GetActions(out actions: array<ref<DeviceAction>>, context: GetActionsContext) -> Bool {
  if !DesossageConfig.Default().worldInteractables.active {
    return false;
  }
  return wrappedMethod(actions, context);
}

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
