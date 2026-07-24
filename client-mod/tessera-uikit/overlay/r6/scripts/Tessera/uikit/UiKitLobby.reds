module Tessera.UiKit

import Codeware.UI.*

// ─────────────────────────────────────────────────────────────────────────────
// LOBBY D'ARRIVÉE — v1 ink (C3/C4), premier écran du parcours joueur (implémentation
// chronologique décidée le 2026-07-23). Design validé : maquette-lobby-arrivee.html (v2, DA
// officielle DIRECTION-ARTISTIQUE.md) + spec flux d'arrivée 2026-07-18.
//
// PÉRIMÈTRE v1 (dev) : l'ÉCRAN seul — cartes de personnages (données locales de démo, la
// tranche A serveur CharacterStore viendra remplacer), sélection au clic (cadre cyan), bouton
// CONNEXION activé par la sélection, carte « Créer » (stub — sonde 1a du creator à venir).
// HORS périmètre v1 : gel GameplayTier/bulle, messages serveur, creator natif.
//
// Véhicule : InGamePopup (Codeware) comme le panneau H2 VALIDÉ EN JEU — vignette ROUGE native
// (exactement la DA du lobby), curseur, input modal, ESC pour fermer (comportement dev ; en
// prod le lobby ne sera pas fermable sans choisir). Toutes les signatures ci-dessous sont
// celles du Codeware.UI.reds local qui compile — zéro devinette.
// ─────────────────────────────────────────────────────────────────────────────
public class TesseraLobbyPopup extends InGamePopup {

  // Cadres (cell_fg) des cartes — re-teintés à la sélection (rouge = structure, cyan = sélection).
  protected let m_frames: array<wref<inkImage>>;
  protected let m_corners: array<wref<inkText>>;
  protected let m_cardNames: array<String>;
  protected let m_selected: Int32;
  protected let m_connect: ref<SimpleButton>;
  protected let m_status: wref<inkText>;

  public func UseCursor() -> Bool {
    return true;
  }

  protected cb func OnCreate() {
    super.OnCreate(); // vignette rouge + m_container centré (~1550x840)
    this.m_selected = -1;

    // SENSATION DE MENU (retour Lucas 2026-07-23) : sans ceci, le lobby flottait au-dessus du
    // gameplay. Voile quasi opaque plein écran INSÉRÉ DERRIÈRE (index 0) la vignette/le contenu —
    // le monde disparaît, comme dans les menus natifs. Le flou + la dilatation du temps
    // d'InGamePopup s'ajoutent par-dessus.
    let root: wref<inkCompoundWidget> = this.GetRootCompoundWidget();
    let backdrop: ref<inkRectangle> = new inkRectangle();
    backdrop.SetName(n"backdrop");
    backdrop.SetAnchor(inkEAnchor.Fill);
    backdrop.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    backdrop.BindProperty(n"tintColor", n"MainColors.Fullscreen_PrimaryBackgroundDarkest");
    backdrop.SetOpacity(1.0); // PLEINEMENT OPAQUE (retour Lucas) — plus aucun résidu du monde
    backdrop.Reparent(root, 0);

    // FOND DE MENU ANIMÉ — reconstruction du « fluff » des menus natifs (retour Lucas : reprendre
    // le fond animé de l'inventaire). La sonde ink (TesseraInkProbe, 2026-07-23) a montré que ces
    // décos (Left_lines/Right_lines : `side_element` empilés + colonnes binaires ; bottom fluff)
    // vivent DANS l'arbre du hub-menu, pas dans un .inkwidget autonome → pas de SpawnFromExternal.
    // On les RECONSTRUIT ici en primitives (rectangles + texte 0/1) — aucune dépendance d'atlas,
    // donc rendu garanti — animées en boucle (opacité ping-pong) pour l'effet « menu vivant ».
    // Barres de flanc RETIRÉES (retour Lucas : pas jolies). On garde le voile + la frise du bas.
    this.BuildBottomFluff(root);

    // Conteneur plus haut/large pour aérer — structure en ZONES ANCRÉES (pas un simple
    // empilement) : barre serveur en haut, cartes centrées, pied de page (notice + CONNEXION)
    // ANCRÉ EN BAS, séparé des cartes — plus proche de la maquette (retour Lucas 2026-07-23).
    this.m_container.SetWidth(1400.0);
    this.m_container.SetHeight(880.0);

    // ---- Barre serveur (haut) : marque à gauche, stats à droite, liseré rouge dessous ----
    // (maquette-lobby-arrivee.html .top — pas de sous-titre : la plaque ci-dessous fait office
    // de contexte, comme dans le vrai menu.)
    let brand: ref<inkHorizontalPanel> = new inkHorizontalPanel();
    brand.SetName(n"brand");
    brand.SetChildMargin(inkMargin(0.0, 0.0, 14.0, 0.0));
    brand.SetAnchor(inkEAnchor.TopLeft);
    brand.SetMargin(inkMargin(60.0, 34.0, 0.0, 0.0));
    brand.Reparent(this.m_container);

    let brandV: ref<inkText> = new inkText();
    brandV.SetName(n"brand_v");
    brandV.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
    brandV.SetFontStyle(n"Bold");
    brandV.SetFontSize(34);
    brandV.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    brandV.BindProperty(n"tintColor", n"MainColors.Blue");
    brandV.SetText("TESSERA");
    brandV.Reparent(brand);

    let brandK: ref<inkText> = new inkText();
    brandK.SetName(n"brand_k");
    brandK.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
    brandK.SetFontStyle(n"Medium");
    brandK.SetFontSize(20);
    brandK.SetLetterCase(textLetterCase.UpperCase);
    brandK.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    brandK.BindProperty(n"tintColor", n"MainColors.White");
    brandK.SetText("// Serveur RP");
    brandK.Reparent(brand);

    let stats: ref<inkHorizontalPanel> = new inkHorizontalPanel();
    stats.SetName(n"stats");
    stats.SetChildMargin(inkMargin(0.0, 0.0, 28.0, 0.0));
    stats.SetAnchor(inkEAnchor.TopRight);
    stats.SetMargin(inkMargin(0.0, 40.0, 60.0, 0.0));
    stats.Reparent(this.m_container);
    this.AddStatText(stats, "EN LIGNE 12/64");
    this.AddStatText(stats, "PING 24 MS");
    this.AddStatText(stats, "tessera-dev-01");

    let topLine: ref<inkRectangle> = new inkRectangle();
    topLine.SetName(n"top_line");
    topLine.SetSize(10.0, 1.0);
    topLine.SetAnchor(inkEAnchor.TopFillHorizontaly);
    topLine.SetMargin(inkMargin(0.0, 92.0, 0.0, 0.0));
    topLine.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    topLine.BindProperty(n"tintColor", n"MainColors.Red");
    topLine.SetOpacity(0.35);
    topLine.Reparent(this.m_container);

    // ---- Plaque de titre « ◁ [Q] — Personnages — [D] ▷ » + ticks de position ----
    this.BuildTitlePlate(this.m_container, "Personnages", 0);

    // ---- Cartes : centrées au milieu de l'écran, indépendantes du pied de page ----
    let row: ref<inkHorizontalPanel> = new inkHorizontalPanel();
    row.SetName(n"cards_row");
    row.SetChildMargin(inkMargin(0.0, 0.0, 26.0, 0.0));
    row.SetAnchor(inkEAnchor.Centered);
    row.SetAnchorPoint(Vector2(0.5, 0.5));
    row.SetMargin(inkMargin(0.0, -20.0, 0.0, 0.0));
    row.Reparent(this.m_container);

    // v1 : données locales de démo — remplacées plus tard par CharacterList (tranche A serveur).
    this.CreateCharacterCard(row, 0, "Vika Moreno", "Nomade · Médic", "Vu il y a 2 j · Watson");
    this.CreateCharacterCard(row, 1, "Dex Carter", "Corpo · NetWatch", "Vu il y a 9 j · City Center");
    this.CreateCreateCard(row);

    // ---- Bandeau notice (liseré cyan, façon notice du jeu — maquette .notice) : centré, ANCRÉ
    // au-dessus du pied. Icône encadrée + texte cyan, réécrits au fil des clics.
    let notice: ref<inkHorizontalPanel> = new inkHorizontalPanel();
    notice.SetName(n"notice");
    notice.SetFitToContent(true);
    notice.SetChildMargin(inkMargin(0.0, 0.0, 14.0, 0.0));
    notice.SetAnchor(inkEAnchor.BottomFillHorizontaly);
    notice.SetHAlign(inkEHorizontalAlign.Center);
    notice.SetMargin(inkMargin(0.0, 0.0, 0.0, 128.0));
    notice.Reparent(this.m_container);

    let noticeBar: ref<inkRectangle> = new inkRectangle();
    noticeBar.SetName(n"notice_bar");
    noticeBar.SetSize(4.0, 34.0);
    noticeBar.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    noticeBar.BindProperty(n"tintColor", n"MainColors.Blue");
    noticeBar.Reparent(notice);

    let noticeIcon: ref<inkCanvas> = new inkCanvas();
    noticeIcon.SetName(n"notice_icon");
    noticeIcon.SetSize(30.0, 30.0);
    let noticeIconFrame: ref<inkImage> = new inkImage();
    noticeIconFrame.SetName(n"frame");
    noticeIconFrame.SetAtlasResource(r"base\\gameplay\\gui\\common\\shapes\\atlas_shapes_sync.inkatlas");
    noticeIconFrame.SetTexturePart(n"cell_fg");
    noticeIconFrame.SetTintColor(ThemeColors.ElectricBlue());
    noticeIconFrame.SetAnchor(inkEAnchor.Fill);
    noticeIconFrame.SetNineSliceScale(true);
    noticeIconFrame.SetNineSliceGrid(inkMargin(0.0, 0.0, 6.0, 0.0));
    noticeIconFrame.Reparent(noticeIcon);
    let noticeIconTxt: ref<inkText> = new inkText();
    noticeIconTxt.SetName(n"tri");
    noticeIconTxt.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
    noticeIconTxt.SetFontStyle(n"Bold");
    noticeIconTxt.SetFontSize(18);
    noticeIconTxt.SetTintColor(ThemeColors.ElectricBlue());
    noticeIconTxt.SetText("▲");
    noticeIconTxt.SetAnchor(inkEAnchor.Centered);
    noticeIconTxt.SetAnchorPoint(Vector2(0.5, 0.5));
    noticeIconTxt.Reparent(noticeIcon);
    noticeIcon.Reparent(notice);

    let status: ref<inkText> = new inkText();
    status.SetName(n"status");
    status.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
    status.SetFontStyle(n"Bold");
    status.SetFontSize(22);
    status.SetLetterCase(textLetterCase.UpperCase);
    status.SetTintColor(ThemeColors.ElectricBlue());
    status.SetText("Sélectionne un personnage pour activer la connexion.");
    status.Reparent(notice);
    this.m_status = status;

    // ---- Pied : hint ESC en bas à gauche, CONNEXION en bas à droite (maquette .foot). ----
    let escGroup: ref<inkHorizontalPanel> = new inkHorizontalPanel();
    escGroup.SetName(n"esc_hint");
    escGroup.SetChildMargin(inkMargin(0.0, 0.0, 10.0, 0.0));
    escGroup.SetAnchor(inkEAnchor.BottomLeft);
    escGroup.SetMargin(inkMargin(60.0, 0.0, 0.0, 44.0));
    escGroup.Reparent(this.m_container);

    let escKey: ref<inkCanvas> = new inkCanvas();
    escKey.SetName(n"esc_key");
    escKey.SetSize(58.0, 28.0);
    let escKeyFrame: ref<inkImage> = new inkImage();
    escKeyFrame.SetName(n"frame");
    escKeyFrame.SetAtlasResource(r"base\\gameplay\\gui\\common\\shapes\\atlas_shapes_sync.inkatlas");
    escKeyFrame.SetTexturePart(n"cell_fg");
    escKeyFrame.SetTintColor(ThemeColors.RedOxide());
    escKeyFrame.SetAnchor(inkEAnchor.Fill);
    escKeyFrame.SetNineSliceScale(true);
    escKeyFrame.SetNineSliceGrid(inkMargin(0.0, 0.0, 6.0, 0.0));
    escKeyFrame.Reparent(escKey);
    let escKeyTxt: ref<inkText> = new inkText();
    escKeyTxt.SetName(n"lbl");
    escKeyTxt.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
    escKeyTxt.SetFontStyle(n"Bold");
    escKeyTxt.SetFontSize(18);
    escKeyTxt.SetTintColor(ThemeColors.Bittersweet());
    escKeyTxt.SetText("ESC");
    escKeyTxt.SetAnchor(inkEAnchor.Centered);
    escKeyTxt.SetAnchorPoint(Vector2(0.5, 0.5));
    escKeyTxt.Reparent(escKey);
    escKey.Reparent(escGroup);

    let escLabel: ref<inkText> = new inkText();
    escLabel.SetName(n"esc_label");
    escLabel.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
    escLabel.SetFontStyle(n"Medium");
    escLabel.SetFontSize(22);
    escLabel.SetTintColor(ThemeColors.Bittersweet());
    escLabel.SetText("Quitter");
    escLabel.Reparent(escGroup);

    // CONNEXION — SimpleButton (validé H2), grisé tant qu'aucun perso n'est choisi.
    this.m_connect = SimpleButton.Create();
    this.m_connect.SetName(n"btn_connect");
    this.m_connect.SetText("CONNEXION");
    this.m_connect.ToggleAnimations(true);
    this.m_connect.ToggleSounds(true);
    this.m_connect.SetDisabled(true);
    this.m_connect.Reparent(this.m_container);
    this.m_connect.GetRootWidget().SetAnchor(inkEAnchor.BottomRight);
    this.m_connect.GetRootWidget().SetMargin(inkMargin(0.0, 0.0, 60.0, 40.0));
    this.m_connect.RegisterToCallback(n"OnBtnClick", this, n"OnConnect");
  }

  // Un texte mono rouge de la barre de stats (« EN LIGNE 12/64 », « PING 24 MS »…).
  private func AddStatText(parent: wref<inkCompoundWidget>, txt: String) {
    let t: ref<inkText> = new inkText();
    t.SetName(n"stat");
    t.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
    t.SetFontStyle(n"Medium");
    t.SetFontSize(20);
    t.SetLetterCase(textLetterCase.UpperCase);
    t.SetTintColor(ThemeColors.Bittersweet());
    t.SetText(txt);
    t.Reparent(parent);
  }

  // Plaque de titre « ◁ [Q] — <titre> — [D] ▷ » + ticks de position sous la plaque (maquette
  // .plate/.ticks — navigation entre catégories, ici une seule page : tick 0 actif).
  private func BuildTitlePlate(parent: wref<inkCompoundWidget>, title: String, activeTick: Int32) {
    let plate: ref<inkCanvas> = new inkCanvas();
    plate.SetName(n"plate");
    plate.SetSize(620.0, 52.0);
    plate.SetAnchor(inkEAnchor.TopCenter);
    plate.SetAnchorPoint(Vector2(0.5, 0.0));
    plate.SetMargin(inkMargin(0.0, 118.0, 0.0, 0.0));
    plate.Reparent(parent);

    let frame: ref<inkImage> = new inkImage();
    frame.SetName(n"frame");
    frame.SetAtlasResource(r"base\\gameplay\\gui\\common\\shapes\\atlas_shapes_sync.inkatlas");
    frame.SetTexturePart(n"cell_fg");
    frame.SetTintColor(ThemeColors.RedOxide());
    frame.SetAnchor(inkEAnchor.Fill);
    frame.SetNineSliceScale(true);
    frame.SetNineSliceGrid(inkMargin(0.0, 0.0, 8.0, 0.0));
    frame.Reparent(plate);

    let row: ref<inkHorizontalPanel> = new inkHorizontalPanel();
    row.SetName(n"row");
    row.SetChildMargin(inkMargin(0.0, 0.0, 16.0, 0.0));
    row.SetAnchor(inkEAnchor.Centered);
    row.SetAnchorPoint(Vector2(0.5, 0.5));
    row.Reparent(plate);

    let arrL: ref<inkText> = new inkText();
    arrL.SetName(n"arrow_l");
    arrL.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
    arrL.SetFontStyle(n"Bold");
    arrL.SetFontSize(20);
    arrL.SetTintColor(ThemeColors.Bittersweet());
    arrL.SetText("◁");
    arrL.Reparent(row);
    this.BuildPlateKey(row, "Q");

    let titleTxt: ref<inkText> = new inkText();
    titleTxt.SetName(n"title");
    titleTxt.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
    titleTxt.SetFontStyle(n"Bold");
    titleTxt.SetFontSize(24);
    titleTxt.SetLetterCase(textLetterCase.UpperCase);
    titleTxt.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    titleTxt.BindProperty(n"tintColor", n"MainColors.White");
    titleTxt.SetText(title);
    titleTxt.Reparent(row);

    this.BuildPlateKey(row, "D");
    let arrR: ref<inkText> = new inkText();
    arrR.SetName(n"arrow_r");
    arrR.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
    arrR.SetFontStyle(n"Bold");
    arrR.SetFontSize(20);
    arrR.SetTintColor(ThemeColors.Bittersweet());
    arrR.SetText("▷");
    arrR.Reparent(row);

    let ticks: ref<inkHorizontalPanel> = new inkHorizontalPanel();
    ticks.SetName(n"ticks");
    ticks.SetSize(160.0, 3.0);
    ticks.SetChildMargin(inkMargin(0.0, 0.0, 6.0, 0.0));
    ticks.SetAnchor(inkEAnchor.TopCenter);
    ticks.SetAnchorPoint(Vector2(0.5, 0.0));
    ticks.SetMargin(inkMargin(0.0, 174.0, 0.0, 0.0));
    ticks.Reparent(parent);
    let i: Int32 = 0;
    while i < 3 {
      let tick: ref<inkRectangle> = new inkRectangle();
      tick.SetName(n"tick");
      tick.SetSize(48.0, 3.0);
      tick.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
      if i == activeTick {
        tick.BindProperty(n"tintColor", n"MainColors.Blue");
      } else {
        tick.BindProperty(n"tintColor", n"MainColors.Red");
        tick.SetOpacity(0.35);
      }
      tick.Reparent(ticks);
      i += 1;
    }
  }

  // Petit encadré touche clavier (« Q », « D »…) réutilisé dans la plaque de titre.
  private func BuildPlateKey(parent: wref<inkCompoundWidget>, letter: String) {
    let key: ref<inkCanvas> = new inkCanvas();
    key.SetName(n"key");
    key.SetSize(30.0, 24.0);
    let frame: ref<inkImage> = new inkImage();
    frame.SetName(n"frame");
    frame.SetAtlasResource(r"base\\gameplay\\gui\\common\\shapes\\atlas_shapes_sync.inkatlas");
    frame.SetTexturePart(n"cell_fg");
    frame.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    frame.BindProperty(n"tintColor", n"MainColors.White");
    frame.SetAnchor(inkEAnchor.Fill);
    frame.SetNineSliceScale(true);
    frame.SetNineSliceGrid(inkMargin(0.0, 0.0, 6.0, 0.0));
    frame.Reparent(key);
    let txt: ref<inkText> = new inkText();
    txt.SetName(n"lbl");
    txt.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
    txt.SetFontStyle(n"Bold");
    txt.SetFontSize(16);
    txt.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    txt.BindProperty(n"tintColor", n"MainColors.White");
    txt.SetText(letter);
    txt.SetAnchor(inkEAnchor.Centered);
    txt.SetAnchorPoint(Vector2(0.5, 0.5));
    txt.Reparent(key);
    key.Reparent(parent);
  }

  // Joue une animation d'opacité en boucle (ping-pong infini) sur un widget — donne l'effet
  // « vivant » du fluff natif sans dépendre d'un atlas. API ink standard vérifiée au dump RTTI.
  private func LoopOpacity(target: wref<inkWidget>, from: Float, to: Float, dur: Float) {
    let def: ref<inkAnimDef> = new inkAnimDef();
    let a: ref<inkAnimTransparency> = new inkAnimTransparency();
    a.SetStartTransparency(from);
    a.SetEndTransparency(to);
    a.SetDuration(dur);
    a.SetType(inkanimInterpolationType.Linear);
    a.SetMode(inkanimInterpolationMode.EasyIn);
    def.AddInterpolator(a);
    let opts: inkAnimOptions;
    opts.loopType = inkanimLoopType.PingPong;
    opts.loopInfinite = true;
    target.PlayAnimationWithOptions(def, opts);
  }

  // Bande de segments animés en bas (façon frise de fluff du bas des menus).
  private func BuildBottomFluff(root: wref<inkCompoundWidget>) {
    let rowp: ref<inkHorizontalPanel> = new inkHorizontalPanel();
    rowp.SetName(n"fluff_bottom");
    rowp.SetAnchor(inkEAnchor.BottomFillHorizontaly);
    rowp.SetMargin(inkMargin(90.0, 0.0, 90.0, 30.0));
    rowp.SetChildMargin(inkMargin(0.0, 0.0, 10.0, 0.0));
    rowp.SetHAlign(inkEHorizontalAlign.Left);
    rowp.Reparent(root, 1);
    let i: Int32 = 0;
    while i < 24 {
      let seg: ref<inkRectangle> = new inkRectangle();
      seg.SetName(n"seg");
      seg.SetSize(16.0, 3.0);
      seg.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
      seg.BindProperty(n"tintColor", n"MainColors.Red");
      seg.SetOpacity(0.5);
      seg.Reparent(rowp);
      i += 1;
    }
    this.LoopOpacity(rowp, 0.2, 0.55, 1.8);
  }

  // Carte de personnage : canvas + cell_bg (fond nine-slice) + cell_fg (cadre, re-teinté à la
  // sélection) + pile de textes en bas. Même vocabulaire d'atlas que SimpleButton (H2).
  private func CreateCharacterCard(parent: wref<inkCompoundWidget>, index: Int32, name: String, role: String, lastSeen: String) {
    let card: ref<inkCanvas> = new inkCanvas();
    card.SetName(StringToName(s"card_\(index)"));
    card.SetSize(300.0, 420.0);
    card.SetInteractive(true);

    let bg: ref<inkImage> = new inkImage();
    bg.SetName(n"bg");
    bg.SetAtlasResource(r"base\\gameplay\\gui\\common\\shapes\\atlas_shapes_sync.inkatlas");
    bg.SetTexturePart(n"cell_bg");
    bg.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    bg.BindProperty(n"tintColor", n"MainColors.Fullscreen_PrimaryBackgroundDarkest");
    bg.SetOpacity(0.85);
    bg.SetAnchor(inkEAnchor.Fill);
    bg.SetNineSliceScale(true);
    bg.SetNineSliceGrid(inkMargin(0.0, 0.0, 10.0, 0.0));
    bg.Reparent(card);

    let frame: ref<inkImage> = new inkImage();
    frame.SetName(n"frame");
    frame.SetAtlasResource(r"base\\gameplay\\gui\\common\\shapes\\atlas_shapes_sync.inkatlas");
    frame.SetTexturePart(n"cell_fg");
    frame.SetTintColor(ThemeColors.RedOxide()); // repos : rouge structure (DA)
    frame.SetAnchor(inkEAnchor.Fill);
    frame.SetNineSliceScale(true);
    frame.SetNineSliceGrid(inkMargin(0.0, 0.0, 10.0, 0.0));
    frame.Reparent(card);
    ArrayPush(this.m_frames, frame);
    ArrayPush(this.m_cardNames, name);

    // Coin « ◤ SÉLECTIONNÉ » — invisible au repos, affiché uniquement sur la carte choisie
    // (maquette .corner : opacity 0 → 1 sur .sel).
    let corner: ref<inkText> = new inkText();
    corner.SetName(n"corner");
    corner.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
    corner.SetFontStyle(n"Bold");
    corner.SetFontSize(16);
    corner.SetTintColor(ThemeColors.ElectricBlue());
    corner.SetText("◤ SÉLECTIONNÉ");
    corner.SetAnchor(inkEAnchor.TopRight);
    corner.SetMargin(inkMargin(0.0, 10.0, 12.0, 0.0));
    corner.SetVisible(false);
    corner.Reparent(card);
    ArrayPush(this.m_corners, corner);

    // Silhouette « doll » — tête + torse + jambes, seule forme humaine qu'on peut se permettre
    // sans portrait (maquette .doll/.body) — occupe la zone entre le cadre et les métadonnées.
    let doll: ref<inkCanvas> = new inkCanvas();
    doll.SetName(n"doll");
    doll.SetSize(70.0, 170.0);
    doll.SetAnchor(inkEAnchor.Centered);
    doll.SetAnchorPoint(Vector2(0.5, 0.5));
    doll.SetMargin(inkMargin(0.0, -30.0, 0.0, 0.0));
    doll.Reparent(card);

    let head: ref<inkCircle> = new inkCircle();
    head.SetName(n"b_h");
    head.SetSize(34.0, 34.0);
    head.SetAnchor(inkEAnchor.TopCenter);
    head.SetAnchorPoint(Vector2(0.5, 0.0));
    head.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    head.BindProperty(n"tintColor", n"MainColors.ReadableMedium");
    head.SetOpacity(0.35);
    head.Reparent(doll);

    let torso: ref<inkRectangle> = new inkRectangle();
    torso.SetName(n"b_t");
    torso.SetSize(54.0, 68.0);
    torso.SetAnchor(inkEAnchor.TopCenter);
    torso.SetAnchorPoint(Vector2(0.5, 0.0));
    torso.SetMargin(inkMargin(0.0, 38.0, 0.0, 0.0));
    torso.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    torso.BindProperty(n"tintColor", n"MainColors.ReadableMedium");
    torso.SetOpacity(0.35);
    torso.Reparent(doll);

    let legL: ref<inkRectangle> = new inkRectangle();
    legL.SetName(n"b_l1");
    legL.SetSize(20.0, 62.0);
    legL.SetAnchor(inkEAnchor.TopLeft);
    legL.SetMargin(inkMargin(6.0, 108.0, 0.0, 0.0));
    legL.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    legL.BindProperty(n"tintColor", n"MainColors.ReadableMedium");
    legL.SetOpacity(0.35);
    legL.Reparent(doll);

    let legR: ref<inkRectangle> = new inkRectangle();
    legR.SetName(n"b_l2");
    legR.SetSize(20.0, 62.0);
    legR.SetAnchor(inkEAnchor.TopLeft);
    legR.SetMargin(inkMargin(44.0, 108.0, 0.0, 0.0));
    legR.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    legR.BindProperty(n"tintColor", n"MainColors.ReadableMedium");
    legR.SetOpacity(0.35);
    legR.Reparent(doll);

    // Pile de textes ancrée en bas de la carte — descendue près du bord (retour Lucas : « baisse
    // les descriptions sous les cartes »).
    let meta: ref<inkVerticalPanel> = new inkVerticalPanel();
    meta.SetName(n"meta");
    meta.SetAnchor(inkEAnchor.BottomFillHorizontaly);
    meta.SetMargin(inkMargin(18.0, 0.0, 18.0, 8.0));
    meta.SetChildMargin(inkMargin(0.0, 2.0, 0.0, 2.0));
    meta.SetFitToContent(true);
    meta.Reparent(card);

    let nameText: ref<inkText> = new inkText();
    nameText.SetName(n"name");
    nameText.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
    nameText.SetFontStyle(n"Bold");
    nameText.SetFontSize(34);
    nameText.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    nameText.BindProperty(n"tintColor", n"MainColors.White");
    nameText.SetText(name);
    nameText.Reparent(meta);

    let roleText: ref<inkText> = new inkText();
    roleText.SetName(n"role");
    roleText.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
    roleText.SetFontStyle(n"Medium");
    roleText.SetFontSize(22);
    roleText.SetLetterCase(textLetterCase.UpperCase);
    roleText.SetTintColor(ThemeColors.Bittersweet());
    roleText.SetText(role);
    roleText.Reparent(meta);

    let seenText: ref<inkText> = new inkText();
    seenText.SetName(n"seen");
    seenText.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
    seenText.SetFontStyle(n"Medium");
    seenText.SetFontSize(20);
    seenText.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    seenText.BindProperty(n"tintColor", n"MainColors.ReadableMedium");
    seenText.SetText(lastSeen);
    seenText.Reparent(meta);

    // Un handler par carte (désambiguïsation sûre sans API de ciblage non vérifiée).
    if index == 0 {
      card.RegisterToCallback(n"OnRelease", this, n"OnCard0Release");
    } else {
      card.RegisterToCallback(n"OnRelease", this, n"OnCard1Release");
    }
    card.Reparent(parent);
  }

  // Carte « Créer un personnage » — stub v1 (le creator natif patché arrive avec la sonde 1a).
  private func CreateCreateCard(parent: wref<inkCompoundWidget>) {
    let card: ref<inkCanvas> = new inkCanvas();
    card.SetName(n"card_create");
    card.SetSize(300.0, 420.0);
    card.SetInteractive(true);

    let bg: ref<inkImage> = new inkImage();
    bg.SetName(n"bg");
    bg.SetAtlasResource(r"base\\gameplay\\gui\\common\\shapes\\atlas_shapes_sync.inkatlas");
    bg.SetTexturePart(n"cell_bg");
    bg.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    bg.BindProperty(n"tintColor", n"MainColors.PanelRed");
    bg.SetOpacity(0.5);
    bg.SetAnchor(inkEAnchor.Fill);
    bg.SetNineSliceScale(true);
    bg.SetNineSliceGrid(inkMargin(0.0, 0.0, 10.0, 0.0));
    bg.Reparent(card);

    let frame: ref<inkImage> = new inkImage();
    frame.SetName(n"frame");
    frame.SetAtlasResource(r"base\\gameplay\\gui\\common\\shapes\\atlas_shapes_sync.inkatlas");
    frame.SetTexturePart(n"cell_fg");
    frame.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    frame.BindProperty(n"tintColor", n"MainColors.Red");
    frame.SetAnchor(inkEAnchor.Fill);
    frame.SetNineSliceScale(true);
    frame.SetNineSliceGrid(inkMargin(0.0, 0.0, 10.0, 0.0));
    frame.Reparent(card);

    let plus: ref<inkText> = new inkText();
    plus.SetName(n"plus");
    plus.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
    plus.SetFontStyle(n"Bold");
    plus.SetFontSize(110);
    plus.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    plus.BindProperty(n"tintColor", n"MainColors.Red");
    plus.SetAnchor(inkEAnchor.Centered);
    plus.SetAnchorPoint(Vector2(0.5, 0.5));
    plus.SetText("+");
    plus.Reparent(card);

    let meta: ref<inkVerticalPanel> = new inkVerticalPanel();
    meta.SetName(n"meta");
    meta.SetAnchor(inkEAnchor.BottomFillHorizontaly);
    meta.SetMargin(inkMargin(18.0, 0.0, 18.0, 20.0));
    meta.SetChildMargin(inkMargin(0.0, 2.0, 0.0, 2.0));
    meta.SetFitToContent(true);
    meta.Reparent(card);

    let label: ref<inkText> = new inkText();
    label.SetName(n"label");
    label.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
    label.SetFontStyle(n"Bold");
    label.SetFontSize(28);
    label.SetTintColor(ThemeColors.Bittersweet());
    label.SetText("Créer un personnage");
    label.Reparent(meta);

    let hint: ref<inkText> = new inkText();
    hint.SetName(n"hint");
    hint.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
    hint.SetFontStyle(n"Medium");
    hint.SetFontSize(20);
    hint.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
    hint.BindProperty(n"tintColor", n"MainColors.ReadableMedium");
    hint.SetText("Creator natif · apparence + nom");
    hint.Reparent(meta);

    card.RegisterToCallback(n"OnRelease", this, n"OnCardCreateRelease");
    card.Reparent(parent);
  }

  protected cb func OnCard0Release(evt: ref<inkPointerEvent>) -> Bool {
    if evt.IsAction(n"click") {
      this.SelectCard(0);
    }
    return true;
  }

  protected cb func OnCard1Release(evt: ref<inkPointerEvent>) -> Bool {
    if evt.IsAction(n"click") {
      this.SelectCard(1);
    }
    return true;
  }

  protected cb func OnCardCreateRelease(evt: ref<inkPointerEvent>) -> Bool {
    if evt.IsAction(n"click") && IsDefined(this.m_status) {
      this.m_status.SetText("Création : creator natif (apparence + nom) — à venir (sonde 1a).");
    }
    return true;
  }

  private func SelectCard(index: Int32) {
    this.m_selected = index;
    let i: Int32 = 0;
    while i < ArraySize(this.m_frames) {
      if IsDefined(this.m_frames[i]) {
        if i == index {
          this.m_frames[i].SetTintColor(ThemeColors.ElectricBlue()); // sélection : cyan (DA)
        } else {
          this.m_frames[i].SetTintColor(ThemeColors.RedOxide());
        }
      }
      if i < ArraySize(this.m_corners) && IsDefined(this.m_corners[i]) {
        this.m_corners[i].SetVisible(i == index); // « ◤ SÉLECTIONNÉ » : uniquement sur la carte choisie
      }
      i += 1;
    }
    if IsDefined(this.m_status) {
      this.m_status.SetText(s"Prêt — personnage : \(this.m_cardNames[index]).");
    }
    this.m_connect.SetDisabled(false);
  }

  protected cb func OnConnect(widget: wref<inkWidget>) -> Bool {
    if this.m_selected >= 0 {
      FTLog(s"[Tessera/UiKit] lobby : CONNEXION avec \(this.m_cardNames[this.m_selected]) (v1 dev — SelectCharacter viendra ici)");
      if IsDefined(this.m_status) {
        this.m_status.SetText("Connexion au serveur… (v1 dev : fermeture du lobby)");
      }
      this.Close();
    }
    return true;
  }

  public static func Show(requester: wref<inkGameController>) {
    let popup: ref<TesseraLobbyPopup> = new TesseraLobbyPopup();
    popup.Open(requester);
  }
}
