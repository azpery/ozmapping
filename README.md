# OZ Mapping

Logiciel de vidéo-mapping live en Rust : calques vidéo superposés, formes de mapping complexes, effets et presets en temps réel, projection sur un second écran.

## Télécharger

Les versions compilées sont sur la page [Releases](https://github.com/azpery/ozmapping/releases/latest) :

- **Windows** : `OZ-Mapping-<version>-windows-x64.zip`. Dézippez puis lancez `OZ Mapping.exe` ; ffmpeg est inclus.
- **macOS** (Apple Silicon et Intel) : `OZ-Mapping-<version>.dmg`. Glissez l'app dans Applications. La première fois, faites clic droit sur l'app puis « Ouvrir » : elle n'est pas signée par un compte Apple Developer. ffmpeg doit être installé : `brew install ffmpeg`.

Le paquet Windows contient ffmpeg, sous licence GPL (fichier `ffmpeg/LICENSE.txt`). Il s'agit des builds [BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds), dont le code source est disponible sur [ffmpeg.org](https://ffmpeg.org/download.html) et sur ce dépôt.

## Prérequis

- Rust (stable)
- ffmpeg et ffprobe : `brew install ffmpeg` sur macOS, `apt install ffmpeg` sur Linux

## Lancer

```sh
cargo run --release
cargo run --release -- clip1.mp4 clip2.mov   # ajoute les vidéos et lance la première
cargo run --release -- spectacle.ozmap       # ouvre un projet
```

## Application macOS

```sh
scripts/build-macos.sh              # Mac de cette architecture (Apple Silicon ici)
scripts/build-macos.sh --universal  # Apple Silicon + Intel
```

Le script produit `dist/OZ Mapping.app` (avec son icône) et `dist/OZ-Mapping-<version>.dmg`. L'app reçoit une signature ad hoc, pas une signature Apple Developer : sur un autre Mac, il faut faire la première fois clic droit sur l'app puis « Ouvrir ». ffmpeg doit être installé sur la machine (`brew install ffmpeg`).

## Windows

Le build Windows se fait sur GitHub Actions (workflow « Build ») : onglet **Actions** > **Build** > **Run workflow**, ou en poussant un tag `v*`, qui publie aussi une release. Le fichier `OZ-Mapping-<version>-windows-x64.zip` contient `OZ Mapping.exe` et ffmpeg (dans `ffmpeg/bin`, build GPL de [BtbN](https://github.com/BtbN/FFmpeg-Builds)) : il suffit de dézipper et de lancer l'exe. Le même workflow produit le .dmg macOS universel.

## Utilisation

**Médias.** Bouton « + Ajouter… » ou glisser-déposer : vidéos (mp4, mkv, mov, avi, webm…) et photos (JPG, PNG, HEIC, WebP, BMP, TIFF). Chaque média a une miniature. Une photo s'affiche comme une image fixe sur son calque, avec fondus, effets, masque, déformation et mix auto ; le bouton « Animer la photo… » du panneau Lecture ouvre ses contours animés.
- **Clic** : pré-visualisation dans le moniteur de gauche, sans toucher à la sortie.
- **Double-clic**, **GO**, touches **1-9** ou **← →** : envoie le média sur le calque sélectionné, avec un fondu enchaîné.

**Annuler.** Cmd+Z (Ctrl+Z sous Windows) annule la dernière modification du projet, Cmd+Maj+Z (ou Ctrl+Y) la rétablit ; aussi dans le menu Projet. Un geste compte pour une étape (glisser un curseur, déplacer un objet de la scène 3D, une frappe continue dans un champ) ; les changements automatiques (mode AUTO, transitions, contrôleur MIDI) n'en créent pas. Dans un champ de texte, Cmd+Z annule la frappe. L'historique repart de zéro à l'ouverture d'un projet.

**Sortie.** La fenêtre « Sortie » s'ouvre au démarrage. Choisissez l'écran du projecteur dans le menu **Écran** de la barre du haut : la sortie s'y place en plein écran. L'écran choisi est retenu pour les prochains lancements. Sinon, déplacez la fenêtre à la main et appuyez sur **F** ou double-cliquez dessus pour le plein écran.

**Calques** (panneau du bas, jusqu'à 8). Chaque calque a sa vidéo, son opacité, son mode de fusion (Normal, Addition, Écran, Produit, Éclaircir, Assombrir), ses effets et sa forme. Le calque du haut de la liste est au premier plan. **Tab** passe au calque suivant.

**Tableau** (à droite des calques). Chaque calque a une ligne de cases ; chaque case garde un média avec tous les réglages du calque : effets, visuel généré, mode AUTO, cadrage, opacité, fusion, déformation, masque, réaction au rythme, mix auto, vitesse et sens de lecture. Tout ce qu'on règle pendant qu'une case joue est gardé dans la case quand on passe à une autre (et à l'enregistrement du projet). Clic sur une case vide : enregistre l'état actuel du calque. Les visuels générés (plasma, spirale, fractale…) sont aussi des médias : la section « Visuels générés » sous la liste les envoie d'un clic sur le calque sélectionné (à la place de sa vidéo) et ils se glissent sur les cases comme une vidéo. Glisser un média de la liste sur une case vide en fait un nouveau preset avec ce média (sans effets) ; sur une case pleine, il remplace son média en gardant ses effets. Clic sur une case pleine : la joue sur son calque (fondu vidéo et transition d'effets) ; en revenant sur une case, sa vidéo reprend là où on l'avait quittée. Clic droit : renommer, remplacer par l'état actuel, vider. Le bouton ▶ en haut d'une colonne joue toutes ses cases d'un coup ; **Enchaîner** passe automatiquement d'une colonne remplie à la suivante, en boucle, avec la durée choisie. Les cases sont enregistrées dans le projet.

**Réglages** (panneau de droite). Les onglets latéraux (Effets, Visuels, Photo, Forme, Mix, Rythme) choisissent une famille de réglages ; des onglets horizontaux en choisissent la section. L'image se cadre dans la surface depuis Forme › Cadrage : Étirer, Ajuster (fit, bandes transparentes) ou Remplir (fill, bords rognés). Si l'effet choisi ne se voit pas (calque masqué, vide, à opacité 0 ou recouvert par un calque opaque), un bandeau propose de l'appliquer sur un calque visible.

**Effets** (onglet Effets). Ils s'appliquent au calque sélectionné.
- 49 presets intégrés : Néon, Kaléido, Mandala, Miroir 4, Tunnel, Rétro VHS, Glitch, 8-bit, Pop art, Arc-en-ciel, Psyché, Rêve, Noir & blanc, Duotone rose, Bleu glacé, Négatif, Strobe, Vortex, Tunnel acide, Liquide, Acide, Mosaïque, Écho, Solarisé, Respiration, Trip, Cube 3D, Sphère, Carte 3D, Cylindre, Relief, Anaglyphe, Drapeau, Thermique, Télé cathodique, Glitch numérique, Pellicule, Onde de choc, Fish-eye, Hexagones, Trame BD, Droste, Zoom radial, ASCII, Tore, Accordéon, Terrain 3D, Pixels 3D, Halo.
- **Couleur** : aussi caméra thermique, flou et halo lumineux (bloom). **Distorsions** : aussi fish-eye, ondes de choc, mosaïque hexagonale, Droste (l'image se répète en spirale infinie qui zoome) et flou de zoom.
- Section **Rétro** : télé cathodique (écran bombé, lignes, phosphores), glitch numérique (bandes et blocs qui sautent), grain de pellicule, trame de points façon BD, rendu en caractères ASCII.
- Section **3D** : l'image posée sur un objet en perspective (plan, cube, sphère, cylindre, tore), ou construit à partir de l'image : **terrain** (la clarté devient un paysage en relief) et **pixels 3D** (une ville de colonnes, chacune de la couleur et de la hauteur de son pixel), avec la hauteur du relief et le nombre de colonnes réglables, avec inclinaison, rotation, rotation et bascule automatiques, distance de la caméra et éclairage ; **relief** (la luminosité donne du volume, éclairé de biais), **anaglyphe** rouge/cyan pour lunettes 3D, **drapeau** (l'image ondule comme un tissu, avec ses ombres). **Accordéon** : l'image pliée en panneaux, en perspective. Autour d'un objet 3D, le calque est transparent.
- Section **Psychédélique** : tourbillon, tunnel (coordonnées polaires qui défilent), mosaïque miroir, distorsion liquide, palette acide, arc-en-ciel spatial, solarisation, pulsation (zoom qui respire), écho (copies superposées).
- Onglet **Visuels** : 34 visuels générés en direct, sans vidéo : plasma, spirale hypnotique, tunnel infini, fractale, mandala, moiré, lampe à lave, nuages fractals, cristaux, op art, hyperespace, soleil rayonnant, ruche, caustiques, aurore boréale, quasicristal, galaxie, labyrinthe, grille rétro, flammes, électricité, gouttes, et 8 visuels **3D** calculés par lancer de rayons : Mandelbulb 3D, éponge de Menger, kaléidoscope 3D (objets qui tournent sous la caméra), anneaux 3D (objets qui tournent sous la caméra), cubes infinis, gyroïde, sphères infinies, colonnes torsadées (vols vers l'avant). Leur Détail règle la finesse (et le coût de calcul). Et **Taches de peinture**, une simulation qui réagit au son : les gouttes tombent sur les temps et les basses (grosses taches qui éclaboussent) et sur les aigus (fines gouttelettes), le volume fait tourbillonner la peinture fraîche ; les couleurs se mélangent comme de vrais pigments (bleu + jaune = vert), la peinture fraîche brille et coule un peu, puis sèche, mate, avec le grain du papier, et s'efface lentement. Sans son, des gouttes tombent quand même, plus calmement. Taille = taille des gouttes, Détail = quantité d'éclaboussures, Vitesse = vitesse de séchage et des remous ; hors des taches, on voit la vidéo du calque. Et deux scènes de dessin animé : **Route cartoon**, un voyage en voiture psychédélique piloté en direct depuis Visuels › Route — virages à gauche ou à droite, virages en S, bosses, creux et montagnes russes qui se forment devant la voiture (ou tirés au hasard), collines, météo (beau temps, nuageux, pluie, orage, brouillard, nuit étoilée, ou chaque réglage à la main : nuages, pluie, éclairs, lumière du jour, brouillard), bouton « Éclair ! » (la foudre tombe aussi sur les temps quand le rythme est actif), voiture (sportive, combi, pick-up, coccinelle, ou aucune) et sa couleur, décor du bord de route (arbres, palmiers, champignons, cactus, cristaux) et dose de délire ; **Voyage temporel**, un vortex d'où jaillissent des horloges affolées et des compteurs d'années (vitesse négative : on remonte le temps ; Détail = nombre d'horloges). Et **Visage qui parle** (Visuels › Visage), un vrai visage humain en 3D sur fond noir, à projeter sur une tête de mannequin ou une boule de cristal (illusion de la voyante) : il parle tout seul (ou sa bouche suit le son de l'onglet Rythme), cligne des yeux, promène son regard, bouge la tête et joue (sourires, sourcils levés, yeux écarquillés : réglage Expressivité). On choisit **Visage seul** (ovale aux bords fondus) ou **Tête entière**, et on règle l'âge (5 à 95 ans : traits, rides et peau changent), le genre, la corpulence, le teint, la couleur des yeux et des sourcils, le maquillage (yeux charbonneux, lèvres sombres), le sourire et les sourcils, avec des personnages tout prêts (voyante, vieil homme, enfant, fantôme…). Cinq rendus : **Projection** (visage très pâle, bleuté et contrasté, fait pour une tête blanche), naturel, spectre, statue, hologramme. **Mon visage (photo)** plaque une photo de face sur la tête, qui parle alors avec ce visage : on la cale sur les yeux et la bouche avec quelques curseurs. La tête, ses déformations et ses textures viennent de MakeHuman (domaine public, voir assets/face/LICENCE.txt) ; `scripts/bake-face.py` les reconstruit. Le bouton Enregistrer de la barre du haut en fait une vidéo. 8 palettes, vitesse, taille, détail et rotation des couleurs. Ils remplacent la vidéo du calque ou s'y mélangent, et passent par tous les effets, le masque et la déformation. Un enchaînement automatique change de motif et de palette au hasard, avec un fondu.
- Dans le même onglet, **Contours animés** : les contours de l'image s'illuminent et bougent, idéal pour donner vie à une photo (marche aussi sur une vidéo ou un visuel). Styles tout prêts (Néon doré, Dessin vivant, Arc-en-ciel, Vagues de lumière), couleur ou arc-en-ciel qui défile, illumination, seuil (ne garder que les contours les plus nets), halo, assombrissement de l'image, vagues de lumière qui partent du centre, ondulation des contours. Les contours s'allument aussi sur les temps (onglet Rythme).
- Dans le même onglet, **Sur la forme du masque** : animations qui épousent le contour du masque du calque — néon (couleur unique ou arc-en-ciel qui tourne), lueur intérieure, comètes et pointillés qui font le tour du contour, lignes concentriques qui avancent vers l'intérieur, échos qui s'éloignent à l'extérieur, onde qui pousse l'image depuis les bords, respiration (la forme grossit et rétrécit). Des presets tout prêts (Néon, Néon arc-en-ciel, Comètes, Pointillés, Ondes, Échos, Aura, Respiration, Onde liquide, Radar, Fête) s'appliquent d'un clic, et « Mémoriser » garde vos propres réglages (clic droit pour supprimer). Elles marchent avec toutes les formes, y compris les SVG et les masques détourés d'une photo, et réagissent au rythme comme les autres effets.
- **Destruction** (onglet Visuels) : la forme du masque se casse et laisse voir les calques du dessous — mur qui s'écroule brique par brique, verre brisé en éclats, combustion en braises, désintégration en poussière, tuiles qui se retournent. Des fissures lumineuses annoncent la cassure, qui part du centre du masque. Réglage manuel de l'avancement, boucle automatique (cassure, pause, reconstruction) ou bouton « Casser ! » pour un coup unique en live. Sans masque, c'est tout le calque qui se casse.
- « Aléatoire » tire une combinaison au hasard.
- **Revenir sans effet** : « Aucun » (touche **0**) retire les effets du calque sélectionné et coupe son mode AUTO. « Sans effets » dans la barre du haut (touche **E**) affiche toutes les vidéos brutes, sur tous les calques, sans perdre les réglages : un second appui les rétablit.
- La durée de « Transition » règle le passage en douceur d'un preset à l'autre.
- « Mémoriser » crée vos propres presets (clic droit pour en supprimer un).
- **AUTO** (touche **A**) : enchaîne automatiquement des effets tirés au hasard sur le calque sélectionné, sans répéter le même deux fois de suite. Un **thème** oriente les choix : Tout, Psychédélique, Couleur (sans déformer l'image), Vitesse, Géométrie, Glitch, Doux ; l'**intensité** dose les effets générés. On règle aussi l'intervalle entre deux changements, la durée du fondu et, pour le thème Tout, la part d'**effets inédits** : le pourcentage de changements qui inventent un nouvel effet au lieu de reprendre un preset (25 % par défaut ; « Mémoriser » permet de garder un effet inventé qui plaît). Le stroboscope est exclu par défaut. « Suivant » force un changement immédiat ; choisir un preset à la main le garde affiché un intervalle complet. Chaque calque a son propre mode Auto, enregistré dans le projet.

**Mix auto** (onglet Mix) : cochez plusieurs vidéos (dans l'onglet ou avec l'icône de bobine sur chaque média), puis activez MIX AUTO. Le calque enchaîne alors des séquences de durée aléatoire : changement de vidéo ou saut à un autre moment, lecture à l'endroit ou **à l'envers**, vitesse variable, coupe franche ou fondu. Chaque réglage (durées, probabilités, vitesses, fondus) est ajustable et enregistré dans le projet ; chaque calque peut avoir son propre mix.

**LEDs WLED** (onglet LEDs) : pilote des bandeaux LED branchés sur des ESP32 avec [WLED](https://kno.wled.ge).
- **Scène** : la salle (largeur, profondeur) et son décor, posés en 3D un peu comme dans SketchUp : **écrans** et surfaces mappées qui affichent l'image de la sortie en direct (toute la sortie ou la zone d'un calque), volumes (boîte, mur), **scène** (largeur, profondeur, hauteur du plateau) **cabine DJ** posée sur la scène, avec ou sans DJ derrière (il hoche la tête en rythme), dont la façade peut afficher l'image, **panneaux superposés** façon « chips » (panneaux de bois ondulés en V, le centre devant et les bords derrière, avec leur nombre, recouvrement, écart entre couches, ondulation, dégradé de couleurs et soleil central ; « Poser les LEDs entre les panneaux » crée une bande cachée derrière le bord de chaque panneau, qui éclaire celui de derrière ; les bandes s'appellent G1, G2… et D1, D2… du centre vers les bords, chaque côté sur le contrôleur choisi, une sortie par bande), et **public** qui danse : une zone où l'on règle le nombre de personnes, leur taille et l'énergie de la danse ; chacun rebondit, se balance et lève les bras sur le tempo de l'onglet Rythme, avec ses couleurs et son propre décalage. Dans la vue 3D : glisser un objet le déplace au sol, Maj+glisser le monte ou le descend, Alt+glisser le tourne (par pas de 15°) ; les cotes de l'objet choisi s'affichent. « Caler l'image des LEDs sur cet écran » : les LEDs prolongent l'image de l'écran. **Groupes d'objets** : Maj+clic sur un deuxième objet (dans la vue 3D ou la liste) les groupe ; un clic sur un membre choisit tout le groupe, qui se déplace, se tourne, se supprime et se redimensionne d'un bloc (fiche du groupe : position, rotation, taille en %, « Modifier un objet seul », « Dégrouper »). Les chips et leurs bandes LED forment un groupe automatiquement.
- **LEDs** : colonnes, bandes, anneaux, matrices (zigzag ou non), spirales, et **tracés libres** : « Dessiner un tracé » pose les points un à un dans la vue 3D, au sol ou sur la face d'un mur, d'un écran ou d'une boîte (Maj : à la hauteur du point précédent ; clic sur le premier point : boucle fermée ; Entrée ou double-clic : terminer ; Retour arrière : retirer le dernier point ; Échap : annuler). Les LEDs se répartissent régulièrement sur le tracé (densités 30, 60, 96, 144 LEDs/m), dont les points restent modifiables. Pour chaque élément : nombre de LEDs, dimensions, position, rotation et sens du câblage. « Exemple : 8 colonnes » pose deux rangées de colonnes. Le **simulateur 3D** (au centre, sous l'aperçu de la sortie) montre les LEDs allumées : glisser pour tourner, clic droit pour déplacer la vue, molette pour zoomer, glisser un élément pour le déplacer au sol ; le survol d'une LED donne son élément, son adresse et sa couleur. L'éclair d'un élément le fait clignoter pour le repérer.
- **Ordre des LEDs** (onglet LEDs) : « Voir le câblage » montre dans la vue 3D le début et le sens de chaque bandeau, son rang sur sa sortie et sa première adresse (« S1·2  #30 »), et en pointillés le fil qui relie la fin d'un bandeau au début du suivant. « Câbler à la souris » : on choisit le contrôleur et la sortie, puis on clique les bandeaux dans l'ordre du fil, près du bout par où arrive le signal ; chaque clic fixe leur place dans la chaîne et leur sens (Échap : terminer). Dans la liste, on glisse un bandeau par sa poignée pour changer sa place ; Cmd+clic (Ctrl+clic sous Windows) et Maj+clic en choisissent plusieurs, qui glissent ensemble, se déplacent ensemble au clavier, et reçoivent d'un coup un sens, une sortie, un groupe de bandeaux, un groupe d'objets ou une suppression. Les boutons monter, descendre et inverser restent disponibles. Les groupes de bandeaux des scénarios se réordonnent aussi en les glissant.
- **Contrôleurs** : autant de WLED que nécessaire, chaque bandeau sur le sien (la liste montre le contrôleur et la sortie de chacun dès qu'il y en a plusieurs ; un contrôleur éteint ou lent à trouver ne retarde pas les autres). Adresse IP de chaque WLED, modèle (ESP32 8 sorties, Gledopto…), pins des sorties et ordre des couleurs. Chaque élément est branché sur une sortie ; les LEDs se suivent sortie par sortie, comme dans WLED. « Lire » récupère la configuration du contrôleur (pins, ordre des couleurs) ; « Envoyer la configuration… » écrit les sorties dans WLED, redémarre le contrôleur et crée un segment par élément (une confirmation est demandée).
- **Scénarios** (Sortie › Scénario) : les bandeaux jouent ensemble, groupe après groupe, comme dans [organic_tears_drops](https://github.com/azpery/organic_tears_drops) : allumage l'un après l'autre, battement de cœur (double battement, et parfois une vague de groupe en groupe), vague, vague de couleurs, arc-en-ciel, comètes qui serpentent de bandeau en bandeau, étincelles, vumètre au son. Calés sur le tempo de l'onglet Rythme (ou 120 BPM), vitesse réglable, enchaînement au hasard avec fondu. Les **groupes de bandeaux** fixent l'ordre : automatiques (paires gauche/droite du centre vers les bords, ou de gauche à droite) ou à la main (ordre, noms, groupe de chaque élément).
- **Sortie** : les LEDs reprennent l'image de la sortie, vue de face, de dessus ou de côté (les cercles sur l'aperçu montrent où chaque LED prend sa couleur), ou une mire de test : couleur unie, arc-en-ciel, chenillard (vérifie le sens du câblage), balayages gauche-droite, bas-haut, avant-fond (vérifient les positions), identification (chaque élément d'une couleur, sa première LED en blanc). Luminosité et images/s réglables ; le blackout éteint aussi les LEDs.
- Rien n'est envoyé tant que le gros bouton « Simulation » n'est pas passé sur **Envoi aux WLED** : on teste d'abord dans le simulateur. Les couleurs partent en DDP (UDP, port 4048). Quand l'envoi est coupé, WLED reprend ses propres effets au bout de quelques secondes. L'installation est enregistrée dans le projet.

**Contrôleur MIDI** (onglet MIDI) : l'Akai MPD218 est dessiné (6 potentiomètres, 16 pads, banques A, B, C) et se connecte tout seul dès qu'il est branché (ou choisir le port). On frappe un pad ou on tourne un potentiomètre : il se sélectionne sur le dessin, il ne reste qu'à choisir sa commande. Pads : case ou colonne du tableau, preset d'effets, effet au hasard, sans effet, média, média suivant/précédent, calque, blackout (bascule ou tant qu'on appuie), écran blanc, sans effets, mire, tap tempo, pause, lecture à l'envers, effets AUTO, « Casser ! », envoi aux WLED, scénario LED. Potentiomètres : luminosité générale, opacité d'un calque, vitesse de lecture, durée des transitions, luminosité des LEDs, ou un réglage d'effet (teinte, zoom, kaléidoscope, tourbillon…). Les pads frappés s'allument sur le dessin, les potentiomètres montrent leur position. « Apprendre » associe un pad du dessin au message réellement envoyé (preset modifié sur le contrôleur, ou autre contrôleur MIDI). Des remplissages tout prêts : pads = colonnes du tableau, presets ou médias, potentiomètres = réglages courants. Les assignations sont gardées d'une session à l'autre.

**Lecture à l'envers** : bouton dans le panneau Lecture, ou touche **R**. La vidéo est décodée par petits morceaux de la fin vers le début (en 1280×720 au maximum).

**Forme** (onglet Forme, touche **M** pour éditer sur l'aperçu ou directement sur la sortie) :
- **Déformation** : une grille de 1×1 (4 coins en perspective) à 8×8 points pour suivre des surfaces courbes. « Aplatir » retire les courbures.
- **Masque** : un polygone (rectangle, cercle, triangle, hexagone, étoile, ou forme libre) avec adoucissement des bords et inversion. Double-clic sur un bord pour ajouter un point, clic droit sur un point pour le supprimer.
- **Masque SVG** : « Importer un SVG… » (ou glisser-déposer un `.svg` sur la fenêtre) reprend la forme d'un logo ou d'un pictogramme comme masque du calque sélectionné : plusieurs formes, trous, courbes et traits sont pris en charge. La forme est centrée sur la sortie, simplifiée (2 000 points au maximum), et ses points restent éditables. Les textes et images contenus dans le SVG sont ignorés : convertissez les textes en tracés dans votre logiciel de dessin.
- **Masque depuis une photo** : « Depuis une photo… » (ou glisser-déposer une image PNG, JPG, HEIC, WebP, BMP ou TIFF quand l'onglet Forme est ouvert ; ailleurs, la photo s'ajoute aux médias) ouvre le détourage. Les photos HEIC (iPhone) sont lues par macOS ou par ffmpeg 7.1+ ; l'orientation des photos de téléphone est respectée.
  - **Sujet (IA)** : un petit réseau de neurones (U²-Net, intégré à l'app, sans connexion) trouve le sujet principal de la photo : bâtiment, objet, personne… Le curseur **Sensibilité** l'élargit ou le resserre.
  - **Cadrer l'objet** : sur une photo chargée (pièce encombrée…), glissez sur l'image pour encadrer ce que vous voulez (un poêle et son conduit, une statue…) ; la détection ne cherche plus que dans le cadre. Maj+glisser ajoute un cadre (plusieurs objets, ou un objet en deux morceaux), « Toute l'image » retire les cadres.
  - **Photo prise de côté** : cochez « Caler sur l'image projetée visible sur la photo ». L'app repère le rectangle projeté (ses 4 coins jaunes restent déplaçables) et place le masque par rapport à lui, perspective corrigée : inutile de photographier depuis le projecteur. « Projeter du blanc » affiche un écran blanc sur la sortie pour prendre la photo.
  - **Couleur du fond** : garde ce qui se distingue de la couleur des bords, idéal pour un logo sur fond uni.
  - Une image déjà détourée (PNG transparent) est prise telle quelle.
  - Retouches : clic pour ajouter une zone de couleur proche (baguette magique), clic droit ou Alt+clic pour la retirer, annuler, inverser, boucher les trous, lisser le contour.
  - **Mapper l'objet** affiche la photo détourée sur le calque, son masque calé dessus, puis passe en mode placement : la croix déplace l'image et le masque ensemble, les coins du cadre les redimensionnent (bouton « Placer l'objet » de l'onglet Photo pour y revenir). Avec une photo calée sur l'image projetée, l'image est remise en perspective. L'onglet **Photo** propose ensuite une vingtaine d'animations en un clic (contours dorés, dessin vivant, arc-en-ciel, aura, néon, comètes, respiration, onde liquide, relief, thermique, glitch, verre brisé, voyage temporel, peinture, lave, plasma…), chacune retouchable dans Effets et Visuels. Le bouton « Photo de l'objet… » du même onglet lance tout le parcours.
  - **Masque seul** en fait un masque vectoriel éditable, sans afficher la photo. La photo entière est ajustée à la sortie : prise depuis le projecteur, le masque tombe pile sur le support.
- **Maj** permet un réglage précis, la croix déplace toute la forme et **G** affiche une mire.

**Rythme** (onglet Rythme) : les effets réagissent au son ou à un tempo.
- **Source** : une entrée audio (micro, carte son) ou un tempo tapé. Pour le son joué par l'ordinateur lui-même, passez par une entrée virtuelle (BlackHole sur Mac, « Mixage stéréo » sous Windows). Le tempo se donne en tapant en rythme sur **TAP** (barre du haut) ou la touche **T**. On peut aussi le saisir, ou le diviser et le doubler.
- **Bande** (entrée audio) : les temps sont détectés dans les **basses** (par défaut : grosse caisse et basse, le plus franc et le plus réactif), sur **tout** le son ou dans les **aigus** (charleston, cymbales). L'analyse se fait toutes les 5 ms, avec un petit tampon audio pour limiter la latence.
- **Tap tempo** : chaque tap donne un temps immédiatement ; le BPM s'affine au fil des taps.
- **Manuel** : chaque appui sur **T** (ou sur PULSE) donne une pulsation, sans tempo qui continue derrière : quand on arrête d'appuyer, les réactions retombent et plus rien ne bouge. Dans ce mode, AUTO et le mix auto gardent leur durée en secondes (les « tous les N temps » ne s'appliquent pas).
- **Réactions toutes prêtes** : Pulsation, Club, Stroboscope, Glitch, Rotation, Miroir, Négatif, Onde de choc, Kaléido, Arc-en-ciel, Douceur.
- **Réaction du calque**, à doser :
  - sur chaque temps : coup de zoom, flash, secousse RVB, saut de teinte, négatif, stroboscope (noir entre les temps), glitch, onde de choc, kaléidoscope, rotation, miroir un temps sur deux ;
  - avec l'intensité de la bande choisie : déformations et saturation qui s'amplifient.
- **Enchaînements sur les temps** : le mode AUTO et le mix auto peuvent changer tous les N temps au lieu d'une durée en secondes.

**Jauge CPU** (en haut à droite) : la charge de l'app et de ses décodeurs ffmpeg, en pourcentage de la machine (vert, orange puis rouge). La partie grise montre la charge du reste de la machine. Le survol donne le détail (nombre de décodeurs, de cœurs).

**Enregistrer** (barre du haut) : enregistre la sortie dans une vidéo MP4 (H.264), du démarrage à l'arrêt — un passage ou tout un spectacle. On choisit la définition (720p, 1080p, 4K, aux proportions de la sortie), les images par seconde (25, 30, 60) et, si on veut, une durée maximale qui arrête l'enregistrement toute seule. Pendant l'enregistrement, le bouton devient « ● REC » avec le temps écoulé : un clic l'arrête. La vidéo est rendue à part, à la définition choisie (indépendamment de la taille des fenêtres) ; elle n'a pas de son. Il faut ffmpeg, comme pour la lecture.

**Projet** (menu Projet). Les fichiers `.ozmap` sont du JSON : médias, calques (avec leur vidéo), effets, formes, presets et réglages. Les médias rangés dans le dossier du projet sont enregistrés en chemins relatifs, ce qui permet de déplacer le dossier. Un « * » dans le titre signale des modifications non enregistrées. La session en cours est aussi restaurée automatiquement au lancement.

| Raccourci | Action |
|-----------|--------|
| Cmd+N / Cmd+O (Ctrl sous Windows) | Nouveau projet / ouvrir un projet |
| Cmd+S / Maj+Cmd+S (Ctrl sous Windows) | Enregistrer / enregistrer sous |
| Espace | Pause / lecture du calque sélectionné |
| 1-9, ← → | Lancer un média sur le calque sélectionné |
| Tab | Calque suivant |
| T | Tap tempo (BPM) |
| R | Lecture à l'envers / à l'endroit (calque sélectionné) |
| 0 | Retirer les effets du calque sélectionné (et couper AUTO) |
| E | Tout afficher sans effets (contournement, réglages conservés) |
| A | Mode Auto (effets aléatoires) sur le calque sélectionné |
| B | Blackout progressif |
| W | Écran blanc (photo de l'image projetée, réglage du projecteur) |
| G | Mire de calibration |
| M | Édition de la forme |
| F / Échap | Plein écran de la sortie / quitter le plein écran |

## Architecture

- `src/video.rs` : décodage via un processus `ffmpeg` qui produit des images RGBA, cadencées par un thread (vitesse, pause, seek, boucle). Photos jouées comme des images fixes (cache des photos décodées). Extraction des miniatures.
- `src/renderer.rs` : composition OpenGL des calques (modes de fusion, fondus), rendu d'un maillage déformé et masque polygonal dans le shader, effets.
- `src/surface.rs` : grille de déformation (perspective et splines de Catmull-Rom), masques, poignées d'édition.
- `src/cutout.rs` : détourage des photos (U²-Net via tract, modèle de couleurs, baguette magique).
- `src/mask.rs` : import SVG, contours d'une image détourée (carrés marchants), rastérisation (tiny-skia) en texture.
- `src/rhythm.rs` : écoute audio (cpal), détection des temps dans les graves, tap tempo, réactions des calques.
- `src/screens.rs` : liste des écrans pour placer la sortie.
- `src/automix.rs` : tirage des séquences du mix auto.
- `src/effects.rs` : paramètres d'effets, interpolation, presets intégrés et aléatoires.
- `src/project.rs` : fichier projet `.ozmap`.
- `src/thumbs.rs` : génération des miniatures en arrière-plan.
- `src/cpu.rs` : mesure de la charge CPU (app + ffmpeg, machine).
- `src/theme.rs`, `src/icons.rs` : apparence (palette, composants) et icônes [Phosphor](https://phosphoricons.com) (police dans `assets/`, licence MIT).
- `assets/models/u2netp.onnx` : modèle [U²-Net](https://github.com/xuebinqin/U-2-Net) de détection du sujet (licence Apache 2.0, `assets/models/LICENSE-U2Net.txt`).
- `src/app.rs` : interface egui (fenêtre de contrôle et fenêtre de sortie).

Limites : pas de son ; les vidéos sont décodées en 1920×1080 au maximum.
