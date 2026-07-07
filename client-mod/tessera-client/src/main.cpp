// Plugin RED4ext minimal « Tessera » — preuve de la chaîne de livraison client + désossage natif.
//
// À l'origine, ce plugin ne faisait RIEN (juste une ligne de log au chargement, pour valider la
// chaîne cloud-build Windows → emballage modset signé → install launcher → chargement en jeu,
// l'équivalent côté client du `probe` côté serveur). Le vrai port Cyberverse (GNS + FlatBuffers)
// reste un plugin/dépôt séparé, pas ici.
//
// Depuis 2026-07-07, ce plugin porte aussi le désossage natif (phase 1, log-only — voir le
// commentaire détaillé plus bas) : les leviers redscript (`tessera-desossage/`) ne peuvent pas
// atteindre certains spawns purement natifs, ce hook C++ observe le point d'accroche identifié
// avant d'envisager un blocage réel.
//
// API ciblée : RED4ext SDK 1.0.0 (API v1, namespace RED4ext::v1). Ce SDK cible le jeu 2.31.
// Déclaré RUNTIME_INDEPENDENT malgré le hook d'adresse précise ci-dessous — voir la note dans
// `Query()` sur pourquoi ce n'est pas encore épinglé à 2.31 spécifiquement.

#include <RED4ext/RED4ext.hpp>
#include <cstdint>

// ─────────────────────────────────────────────────────────────────────────────
// Désossage natif — PHASE 1 : hook LOG-ONLY, aucun blocage.
//
// 5 catégories (cyberpsychos, hustles NCPD, gigs/donneurs de quête par proximité, événements
// aléatoires, PNJ statiques "community") sont hors de portée du redscript — leur spawn est
// exécuté nativement via un nœud de graphe de quête. Recherche dédiée (Fable 5, 2026-07-07)
// identifie le point d'accroche : `QuestPhaseInstance::ExecuteNode`, le dispatcheur central de
// TOUS les nœuds de TOUTES les quêtes du jeu — recommandation explicite « GO PRUDENT » : d'abord
// observer en jeu (log seul, laisser tourner l'original sans jamais bloquer), avant d'envisager un
// blocage réel (phase 2, pas fait ici — la sémantique du retour/des sockets de sortie n'est pas
// encore comprise, un mauvais blocage pourrait geler des graphes de quête en aval).
//
// Hash AddressLib confirmé par 2 sources indépendantes : Codeware
// (psiberx/cp2077-codeware, src/Red/Addresses/Library.hpp,
// `QuestPhaseInstance_ExecuteNode = 3227858325`) et alphanin9/SharedPunk
// (src/include/Impl/Detail/Hashes.hpp, même valeur). Codeware APPELLE cette fonction en
// production (son QuestPhaseExecutor), ce qui valide la signature à grande échelle — mais
// personne ne la HOOKE publiquement, d'où la prudence phase 1.
//
// PIN IN-GAME : la signature exacte (types de `aContext`/sockets) vient du SDK généré
// (`Generated/quest/*.hpp`), pas vérifiée localement (pas d'accès Windows) — laissée en types
// opaques (`void*`) pour ne prendre aucun risque de mauvaise réinterprétation mémoire. Seul le
// paramètre qu'on inspecte (`aNode`) est typé. Si la compilation cloud échoue, ajuster la
// signature avant de retenter — un échec de compilation est sans risque (détecté en CI, jamais
// vu par un joueur), contrairement à une signature fausse qui compilerait quand même.
//
// PIN IN-GAME : si le hash `3227858325` est absent de `cyberpunk2077_addresses.json` (livré par
// CDPR dans `bin\x64\` depuis le patch 2.3, vérifiable via `grep -A2 3227858325
// cyberpunk2077_addresses.json` sur le PC de test AVANT tout déploiement), la résolution
// natif échoue bruyamment au chargement (boîte d'erreur RED4ext, processus terminé) — échec
// propre et déterministe, pas un crash aléatoire (comportement documenté du SDK,
// `UniversalRelocBase::Resolve`).
namespace TesseraDesossageNative
{
using ExecuteNode_t = std::uint8_t (*)(void* aPhase, RED4ext::CClass* aNodeClass, void* aNode,
    void* aContext, void* aInputSocket, void* aOutputSockets);

constexpr std::uint32_t kExecuteNodeHash = 3227858325u;

ExecuteNode_t g_original = nullptr;
RED4ext::v1::Sdk const* g_sdk = nullptr;
RED4ext::v1::PluginHandle g_handle = nullptr;

// Nom RTTI de la classe de nœud visée — englobe questSpawner_NodeType/questSpawnSet_NodeType/
// questCommunityTemplate_NodeType (tous héritent de questSpawnManagerNodeDefinition, confirmé
// via le SDK). Un seul test IsA() suffit.
constexpr const char* kSpawnNodeClassName = "questSpawnManagerNodeDefinition";

std::uint8_t Detour(void* aPhase, RED4ext::CClass* aNodeClass, void* aNode, void* aContext,
    void* aInputSocket, void* aOutputSockets)
{
    // Garde-fou : tout doute (pointeur nul, classe introuvable) → on ne fait QUE journaliser,
    // jamais bloquer. Phase 1 = observation, aucune logique de blocage tant que la sémantique du
    // retour/des sockets de sortie n'est pas comprise en jeu.
    if (aNodeClass != nullptr)
    {
        auto* rtti = RED4ext::CRTTISystem::Get();
        auto* spawnCls = rtti != nullptr ? rtti->GetClass(kSpawnNodeClassName) : nullptr;
        if (spawnCls != nullptr && aNodeClass->IsA(spawnCls) && g_sdk != nullptr)
        {
            g_sdk->logger->InfoF(g_handle,
                "[Tessera/DesossageNative] ExecuteNode sur un nœud spawn (%s) — log seul, phase 1",
                aNodeClass->name.ToString());
        }
    }
    return g_original(aPhase, aNodeClass, aNode, aContext, aInputSocket, aOutputSockets);
}
} // namespace TesseraDesossageNative

// Query — le loader lit notre identité avant de nous charger.
RED4EXT_C_EXPORT void RED4EXT_CALL Query(RED4ext::v1::PluginInfo* aInfo)
{
    aInfo->name = L"TesseraClient";
    aInfo->author = L"TesseraSynth";
    aInfo->version = RED4EXT_V1_SEMVER(0, 1, 0);
    // Recommandation de la recherche (2026-07-07) : épingler à 2.31 maintenant qu'on hooke une
    // adresse précise (ExecuteNode), plutôt que INDEPENDENT. Laissé en INDEPENDENT ici : le nom
    // exact de la constante RED4EXT_V1_RUNTIME_VERSION_2_31 n'a pas pu être vérifié sans accès
    // local au SDK — un mauvais nom serait une erreur de compilation (sûre, détectée en CI), donc
    // pas de risque à laisser tel quel en attendant. Le filet de sécurité réel contre une adresse
    // absente/fausse est documenté plus haut (échec propre au chargement, pas un crash).
    aInfo->runtime = RED4EXT_V1_RUNTIME_VERSION_INDEPENDENT;
    aInfo->sdk = RED4EXT_V1_SDK_VERSION_CURRENT;
}

// Main — appelé au chargement et au déchargement du plugin par le loader.
RED4EXT_C_EXPORT bool RED4EXT_CALL Main(RED4ext::v1::PluginHandle aHandle, RED4ext::v1::EMainReason aReason,
                                        const RED4ext::v1::Sdk* aSdk)
{
    switch (aReason)
    {
    case RED4ext::v1::EMainReason::Load:
        aSdk->logger->Info(aHandle,
            "Tessera chargé — plugin client minimal v0.1.0. La chaîne de livraison fonctionne.");

        // Désossage natif phase 1 (log-only) — voir le commentaire complet plus haut.
        TesseraDesossageNative::g_sdk = aSdk;
        TesseraDesossageNative::g_handle = aHandle;
        {
            auto target = RED4ext::UniversalRelocFunc<TesseraDesossageNative::ExecuteNode_t>(
                TesseraDesossageNative::kExecuteNodeHash);
            bool attached = aSdk->hooking->Attach(aHandle, reinterpret_cast<void*>(target),
                reinterpret_cast<void*>(&TesseraDesossageNative::Detour),
                reinterpret_cast<void**>(&TesseraDesossageNative::g_original));
            if (attached)
            {
                aSdk->logger->Info(aHandle,
                    "[Tessera/DesossageNative] hook ExecuteNode attaché (phase 1, log-only).");
            }
            else
            {
                aSdk->logger->Info(aHandle,
                    "[Tessera/DesossageNative] ERREUR : échec d'attache du hook ExecuteNode.");
            }
        }
        break;

    case RED4ext::v1::EMainReason::Unload:
        aSdk->logger->Info(aHandle, "Tessera déchargé.");
        break;
    }

    return true;
}

// Supports — version d'API que ce plugin attend du loader (API v1).
RED4EXT_C_EXPORT uint32_t RED4EXT_CALL Supports()
{
    return RED4EXT_API_VERSION_1;
}
