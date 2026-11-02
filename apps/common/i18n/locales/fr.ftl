language-name = français 
is_rtl = false

settings = Paramètres
settings_layout = Navigation des paramètres
settings_layout_cd = CD
settings_layout_topbar = Barre supérieure
theme_editor = Éditeur de thème
appearance = Apparence
language = Langage 
library = Librairie
playlists = Playlists
album = Album
release_single = Single
release_ep = EP
release_audiobook = Audiobook
artist = Artiste
home = Accueil 
search = Chercher
favorites = Favoris
radio = Radio
discover = Découvrir
logs = Logs
activity = Activité
general = General
folder_required = Choisissez au moins un dossier
service = Service: { $name }
connected = ● Connecter
disconnected = ● déconnecter
connecting = ● Connexion…
reconnect = Reconnexion
remove = Enlever
add = Ajouter
add_folder = Ajouter un dossier
no_music_folders = Aucun dossier de musique configuré
remote_music_folders = Dossiers de musique
source_account = Compte
source_account_help = Chaque compte a sa propre bibliothèque. Changer de compte la resynchronise.
source_accounts_failed = Impossible de charger les comptes : { $error }
parent_folder = Remonter
use_this_folder = Ajouter ce dossier
no_subfolders = Aucun sous-dossier
loading_folders = Chargement des dossiers…
folder_browse_failed = Impossible de lister les dossiers : { $error }
add_source = Ajouter une source
connect_to_lastfm = Se connecter à Last.fm
connect_to_librefm = Se connecter à Libre.fm
lastfm_connected = Connecté à Last.fm
librefm_connected = Connecté à Libre.fm
server_details = Détails du serveur
source_name = Nom
server_url = URL Serveur
choose_service = Choisir Un Service
cancel = Annuler
login = Se Connecter
logging_in = Connexion...
login_to_service = Connecter a { $service }
username = Nom Utilisateur
password = Mot de Passe
sign_in_method = How to sign in
sign_in_with_browser = Sign in with a browser
sign_in_anonymously = Continue without signing in
sign_in_browser = Browser
sign_in_browser_auto = Navigateur par défaut du système
yt_anonymous_help = Anonymous mode browses and plays public music, but has no library, no liked songs and no Discover.
soundcloud_sign_in_help = Signing in opens a browser window; kopuz keeps only the session it produces.
player_settings = Parametre du Player
discord_presence = Discord Presence
connectivity = Connectivité
integration_connect = Connect to { $name }
integration_connected = { $name } connected
integration_connecting = Connecting…
integration_browser_title = Use a signed-in browser
integration_browser_profile = { $name } ({ $browser })
integration_browser_none = No browser profiles found. Paste your token above instead.
discord_presence_paused = Afficher le statut Discord en pause
discord_presence_source = Afficher la source multimédia dans le statut Discord
reduce_animations = Reduire Les Animation
fullscreen_use_player_bar = Utiliser la barre de lecture comme commandes en plein écran
cover_art_background = Pochette en arrière-plan
cover_art_darkening = Assombrissement de la pochette
cover_art_blur = Flou de la pochette
custom_background = Arrière-plan personnalisé
choose_image = Choisir une image
custom_font = Police personnalisée
choose_font = Choisir une police
live_theme = Matugen / Pywal (en direct)
live_theme_file = Fichier de palette
choose_palette = Choisir une palette
jump_to_current_song = Aller à la chanson en cours
hide_side_panel = Masquer le panneau latéral
show_side_panel = Afficher le panneau latéral
show_source_toggle = Afficheur De Source Toggle
show_row_images = Afficher les images des lignes
titlebar_mode = Barre de titre
titlebar_custom = Personnalisé (dessiné par l'app)
titlebar_system = Décorations système
titlebar_off = Sans barre de titre
volume = Volume
source = Source
sources = Sources
manage_sources = Manage sources
source_on_this_device = On this device
server = Serveur
downloader = Téléchargements

# UI Actions & Buttons
add_to_favorites = Ajouter au Favoris
remove_from_favorites = Supprimer Des Favoris
dont_recommend = Ne plus recommander
add_to_queue = Ajouter à la file d'attente
play_next = Lire ensuite
add_to_playlist = Ajouter a la playlist
remove_from_playlist = Supprimer De La Playlist
start_radio = Lancer La Radio
radio_starting = Lancement de la radio…
radio_empty = Aucun titre similaire trouvé
radio_failed = Impossible de lancer la radio
remove_from_queue = Retirer de la file d'attente
downloading = Téléchargement…
downloading_progress = Downloading { $done } / { $total }
more_actions_for = Plus d'actions pour { $name }
share_musicbrainz = Partager (MusicBrainz)
share_link_copied = Lien MusicBrainz copié
delete = Supprimer
delete_from_device = Supprimer de l'appareil
delete_song = Supprimer le son
delete_album = Supprimer L'album
add_all_to_queue = Ajouter tout à la file d'attente
add_all_to_playlist = Ajouter a la playlist
select_all = Tout sélectionner
select_all_tracks = Sélectionner toutes les pistes
deselect_all_tracks = Désélectionner toutes les pistes
remove_from_cache = Supprimer du cache
create = Crée
save = Sauvegarder
enabled = Activé
disabled = Désactivé
play = Lecture
shuffle = Aléatoire

# Navigation & Headers
tracks = Tracks
albums = Albums
artists = Artiste
all_albums = Tout les Albums
your_library = Votre Librairie
listening_logs = Logs D'écoute
browse_genres = Parcourir les genres
top_artists = Top Artists
new_releases = Nouvelle Sortie

# Navigation Buttons
up_next = A suivre
lyrics = Paroles
track_version = Version
track_version_song = Titre
track_version_video = Vidéo
music_video = Clip vidéo

# Player/Media
loading_lyrics = Chargement des Paroles...
lyrics_not_found = Paroles non trouvée
instrumental_break = Instrumental
no_previous_songs = Aucun sons ne suit
playlist_track_count =
    { $count ->
        [one] { $count } track
       *[other] { $count } tracks
    }
playlist_folder_count =
    { $count ->
        [one] { $count } playlist
       *[other] { $count } playlists
    }
playlists_synced_count =
    { $count ->
        [one] { $count } playlist synchronisée
       *[other] { $count } playlists synchronisées
    }
playlists_loading_tracks = Chargement des titres — playlists { $done } / { $count } (reste { $remaining })
music_playlist_count =
    { $count ->
        [one] Music • { $count } track
       *[other] Music • { $count } tracks
    }
showcase_song_count =
    { $count ->
        [one] { $count } son
       *[other] { $count } sons
    }

# Modal Titles
add_playlist = Add playlist
create_new_playlist = Crée une nouvelle Playlist
playlist_name_placeholder = Nom de la Playlist
playlist_name_input = Nom de la Playlist
browser_sign_in_needs_host = La connexion via le navigateur nécessite un accès à l'hôte. Exécutez la commande ci-dessous et redémarrez kopuz.
browser_playback_needs_host = La lecture s'exécute dans un navigateur sur l'hôte, auquel ce Flatpak n'a pas encore accès. Exécutez la commande ci-dessous et redémarrez kopuz.
experimental = experimental
copy = Copy
media_server = Media Server
saved_servers = Serveurs enregistrés
switch_to_server = Changer
active_server = Actif
no_saved_servers = Aucun serveur enregistré
jellyfin = Jellyfin
subsonic = Subsonic
custom_manual = Custom (manual API)
server_name_optional = Serveur Nom (optional)
server_url_placeholder = http://localhost:8096
subsonic_url_placeholder = http://localhost:4533
nextcloud_url_placeholder = https://cloud.example.com
nextcloud_help = Point this at your Nextcloud, then sign in with an app password. Your music folders are picked after it connects.
apple_music_storefront = Storefront
apple_music_language = Language
apple_music_paste_token = Paste a media-user-token instead
apple_music_token = Media user token
spotify_client_id = Client ID
spotify_client_id_help = Create an app at developer.spotify.com, add http://127.0.0.1:8898/callback as its redirect URI, and paste its Client ID here. Playback needs a Premium account.
change = Changer
heart_track_to_add = Ajoutez ce morceau aux favoris pendant la lecture pour l’ajouter ici.
heart_track_to_add_server = Ajoutez ce morceau aux favoris pendant la lecture pour l’ajouter ici et syncronisée le au Serveur.

# Search & Placeholders
search_placeholder = Chercher Des Artistes , albums ou tracks..
quick_search_placeholder = Tapez pour rechercher, Entrée pour lire…
no_results_found = Aucun Résultat pour "{ $query }"
listenbrainz_token_placeholder = Entrez vous token ListenBrainz 
lastfm_api_key_placeholder = Entrez votre clé API Last.fm
lastfm_api_key_label = API key
lastfm_api_secret_placeholder = Entrez votre secret API Last.fm
lastfm_api_secret_label = API secret

# Empty States
album_not_found = Album non trouver
artist_not_found = Artiste introuvable
artist_load_failed = Impossible de charger cet artiste : { $error }
playlist_not_found = Playlist non trouver
no_playlists_found = No playlists non trouver
no_playlists_yet = Aucune playlist pour l'instant ? Ajouter en une avec des sons de votre librairie!
add_music_to_get_started = Ajouter une musique pour commencer.

# Data Labels
title = Titre
time = Temp
genre = Genre
plays = écoutes
track_count =
    { $count ->
        [one] { $count } track
       *[other] { $count } tracks
    }
songs = sons
min = Minute

# Error Messages
invalid_server_url = URL Serveur Invalid
server_name_required = Le nom du serveur est requis
username_and_password_required = Nom d'utilisateur Et Mot de passe Réquis !
login_failed = Connexion fail: { $error }
apple_music_storefront_required = Saisissez un identifiant de boutique Apple Music.
apple_music_token_required = Saisissez votre media-user-token Apple Music, ou connectez-vous via le navigateur.
spotify_client_id_required = Saisissez le Client ID de votre application Spotify (à créer sur developer.spotify.com).
signin_failed = Échec de la connexion : { $error }
error_server_not_configured = Serveur non configurée ou credential manquant
error_fetch_songs = Échec de récupération des morceaux de l’album '{ $album_id }': { $error }
unsupported_provider = { $service } Est configurée , mes cette page n'est pas disponible pour le provider
unsupported_provider_desc = La navigation propre à { $service } sera bientôt ajoutée grâce à la nouvelle couche serveur.

# Additional Keys
no_genres_found = Aucun Genre Trouvée dans votre librairie
jump_back_in = Resauter dans
by_artist = Par
by_artist_full = Par { $artist }
start_listening = Commencer a écouter
listen_now = En écoute
no_albums_found = Aucun Albums trouver dans votre librairie.
no_favorites = Aucun favoris pour l'instant.
no_tracks_found = Aucune tracks trouvée.
unknown_artist = Artiste Inconnus.
unknown_album = Album Inconnus;
unknown_title = Titre Inconnus.
unknown_genre = Inconnus.
no_tracks_in_library = Aucune tracks trouver dans votre librairie.
no_songs_here = Aucun sons ici.
syncing_with_server = Syncronisation avec le Serveur.
yt_syncing_progress =
    { $count ->
        [one] Synchronisation — { $count } piste chargée
       *[other] Synchronisation — { $count } pistes chargées
    }
yt_synced_total =
    { $count ->
        [one] { $count } piste aimée synchronisée
       *[other] { $count } pistes aimées synchronisées
    }
refresh = Actualiser
most_played_tracks = Votre tracks les plus écouter.
no_more_songs = Plus aucun sons dans la liste.
server_playlist = Serveur Playlist
folder_playlists = Playlists de dossiers
shuffle_on = Aléatoire: Activée
shuffle_off = Aléatoire: Désactivé
repeat_off = Repeter: Désactivé
repeat_queue = Répeter: En attente
repeat_track = Répeter: Track
rescan_library = Rescanner la Librairie
refresh_music_library = Rafraichir la music librairie
listenbrainz = ListenBrainz
lastfm = Last.fm
librefm = Libre.fm
enable_tracing = Activer le traçage des performances
tracing_warning = Ajoute une surcharge et agrandit le fichier de trace ; prend effet après un redémarrage.
export_logs = Exporter les logs
open_logs_folder = Ouvrir le dossier des logs
album_art_gradient = Album art gradient
default_theme = Default
gruvbox_material = Gruvbox Material
gruvbox_classic = Gruvbox Classic
gruvbox_dark_soft = Gruvbox Dark Soft
dracula = Dracula
nord = Nord
catppuccin_mocha = Catppuccin Mocha
ef_night = Ef Night
ayu_dark = Ayu Dark
ayu_mirage = Ayu Mirage
vague = Vague
one_dark_pro = One Dark Pro
osmium = Osmium
kanagawa_dragon = Kanagawa Dragon
everforest = Everforest
rosepine = Rosé Pine
default_light = Default Light
amoled_black = AMOLED Black
catppuccin_latte = Catppuccin Latte
rosepine_dawn = Rosé Pine Dawn
everforest_light = Everforest Light
ayu_light = Ayu Light
one_light = One Light
gruvbox_light_soft = Gruvbox Light Soft

# Theme Editor
new_theme = Nouveau Theme
theme_name = Nom du theme
my_custom_theme = Mon thème personnalisé
theme_group_dynamic = Dynamic
theme_group_dark = Sombre
theme_group_light = Clair
theme_group_custom = Custom
colors = Couleurs
preview = Prévisualisation
save_theme = Sauvegarder le thème
track_title = Titre de la piste

# Color Labels
bg = Fond
raised = Raised Surface
surface = Surface
text = Text
text-muted = Text Muted
accent = Accent
accent-soft = Accent Soft
accent-alt = Accent Alt
accent-deep = Accent Deep
highlight = Highlight
highlight-dark = Highlight Dark
progress = Progress
danger = Danger

downloader_title = Téléchargements
downloader_subtitle = YouTube · YouTube Music
downloader_options = Options
downloader_url_placeholder = Titre, artiste ou album, ou collez un lien…
downloader_download = Télécharger
downloader_output_dir_placeholder = Dossier de sortie (par défaut : dossier Musique)
downloader_pick_folder = Choisir un dossier
downloader_clear_history = Effacer l'historique
downloader_empty_state = Cherchez un morceau pour commencer, ou collez un lien YouTube
downloader_section_embed = Intégrer dans le fichier
downloader_section_write = Écrire des fichiers séparés
downloader_section_behavior = Comportement
downloader_embed_metadata = Intégrer les métadonnées
downloader_embed_thumbnail = Intégrer la miniature
downloader_write_thumbnail = Écrire la miniature
downloader_organize_by_album = Classer par album
downloader_overwrite_existing = Écraser les fichiers existants
downloader_format_best_audio = Meilleur Audio
downloader_format_mp3 = MP3
downloader_format_flac = FLAC
downloader_format_opus = OPUS
downloader_format_wav = WAV
downloader_needs_ffmpeg = Nécessite ffmpeg
downloader_status_downloading = { $percent }%
downloader_status_processing = Traitement en cours...
downloader_status_completed = Terminé
downloader_status_failed = Failed
downloader_status_waiting = En attente...

featured_album = Album en vedette
minimize = Réduire
maximize = Agrandir
close = Fermer
mini_player = Mini-lecteur
restore_full_player = Restaurer le lecteur complet
delete_playlist = Supprimer la playlist
delete_folder = Supprimer le dossier
rename_playlist = Renommer la playlist
rename_folder = Renommer le dossier
equalizer = Égaliseur
eq_preset = Préréglage
eq_preamp = Préampli
eq_preamp_desc = Marge avant la courbe EQ
eq_graph_hint = Ajuster une bande fait passer le préréglage sur Personnalisé.
eq_preset_flat = Plat
eq_preset_bass_boost = Amplification des basses
eq_preset_treble_boost = Amplification des aigus
eq_preset_vocal_boost = Amplification des voix
eq_preset_loudness = Loudness
eq_preset_custom = Personnalisé
replay_gain = Normalisation du volume
replay_gain_mode_off = Désactivée
replay_gain_mode_track = Piste
replay_gain_mode_album = Album
replay_gain_mode_auto = Automatique
replay_gain_prevent_clipping = Éviter la saturation
replay_gain_preamp = Préamplification
replay_gain_fallback = Gain par défaut
replay_gain_youtube_loudness = Normaliser YouTube Music
move_to_folder = Déplacer vers le dossier
no_folders_yet = Aucun dossier pour le moment.
folder_name = Nom du dossier...
new_folder = Nouveau dossier
remove_from_folder = Retirer du dossier
back_behavior = Comportement du bouton Retour
back_behavior_rewind = REMBOBINER → PRÉCÉDENT
back_behavior_always_prev = TOUJOURS PRÉCÉDENT
folder_playlist = Dossier

channel_mode = Mode de canal
channel_mode_stereo = Stéréo
channel_mode_mono = Mono
channel_mode_left_only = Gauche uniquement
channel_mode_right_only = Droite uniquement
channel_mode_swap_left_right = Inverser G/D
device_change_behavior = Au changement de périphérique
device_change_resume = Continuer la lecture
device_change_pause = Mettre en pause
sample_rate_mode = Fréquence d'échantillonnage de sortie
sample_rate_mode_system = Suivre le système
sample_rate_mode_source = Adapter à la piste
settings_group_streaming = Streaming
stream_quality = Qualité du streaming
stream_quality_low = Basse
stream_quality_normal = Normale
stream_quality_high = Haute
autoplay_radio = Lancer une radio à la fin de la file d’attente
skip_explicit = Ignorer les titres explicites
pause_watch_history = Suspendre l’historique des vidéos regardées

crossfade = Fondu enchaîné
crossfade_off = Désactivé

continue_listening = Continuer l'écoute
music = Musique
made_for_you = Fait pour vous
recently_added = Ajouté récemment
customize_home = Personnaliser l'accueil
done_editing = Terminé
reset_home = Réinitialiser
hide_section = Masquer
show_section = Afficher
move_up = Monter
move_down = Descendre
home_section_catalog = Recommandations
home_section_hero = En vedette
home_section_continue_listening = Continuer l'écoute
home_section_listen_now = Écouter maintenant
home_section_top_artists = Meilleurs artistes
home_section_new_releases = Nouveautés
home_section_made_for_you = Fait pour vous
home_section_recently_added = Ajouté récemment
home_section_playlists = Playlists
layout_cards = Cartes
layout_list = Liste
listen_now_layout = Disposition
ui_style = Style d'interface
ui_normal = Normal
ui_vaxry = Vaxry
ui_material3 = Material 3
system_colors = Couleurs du système
player_bar_position = Position de la barre du lecteur
position_bottom = Bas
position_top = Haut

metadata = Métadonnées
auto_fetch_covers = Récupérer automatiquement les pochettes
prefer_local_lyrics = Préférer les paroles locales (.lrc / intégrées, sans repli en ligne)
enable_musixmatch_lyrics = Utiliser Musixmatch comme source de paroles de secours
lyrics_depth_blur = Flouter les lignes de paroles selon leur distance par rapport à la ligne active
lyrics_depth_blur_strength = Intensité du flou des paroles
lyrics_offset = Décalage des paroles
lyrics_offset_auto = Déterminer le décalage des paroles automatiquement
cover_fetch_strategy = Stratégie de récupération des pochettes
musicbrainz_first = MusicBrainz en premier
lastfm_first = Last.fm en premier
musicbrainz_only = MusicBrainz uniquement
lastfm_only = Last.fm uniquement
remove_cover = Retirer la pochette

# Radio Page
radio_filter_stations = Filtrer les stations…
radio_search_stations = Rechercher des stations…
radio_selected = Sélection
radio_top_stations = Stations populaires
radio_search_results = Résultats de recherche
radio_powered_by = Propulsé par radio-browser.info
radio_loading_stations = Chargement des stations… 
radio_search_failed = Impossible de joindre radio-browser.info!
radio_subtitle = Écoutez des stations de radio Internet en direct
radio_no_stations_match = Aucune station ne correspond à votre recherche
radio_station_col = Station
radio_description_col = Description
radio_streams_col = Flux
radio_listen_moe_desc = Musique d'anime et de pop coréenne, 24/7.
radio_j1_desc = Succès actuels et chansons classiques du Japon.
radio_doujinstyle_desc = Musique doujin, bandes originales de jeux vidéo et plus.
radio_vocaloid_desc = Succès Vocaloid du Japon. Nous jouons de tout !
radio_asiadreamradio_desc = Le cœur de la musique J-Pop - J-Pop, J-Rock, J-HipHop, J-Jazz, classiques.
radio_live_stream = Flux en direct

auto_check_updates = Vérifier automatiquement les mises à jour
minimize_to_tray = Réduire dans la barre d'état système
update_available = Mise à jour disponible
update_banner_message = Kopuz { $version } est disponible.
view_release = Voir la version
update_install = Mettre à jour et redémarrer
update_downloading = Téléchargement de Kopuz { $version }…
update_restarting = Redémarrage pour terminer la mise à jour…
update_failed = Impossible d'installer Kopuz { $version }.
update_retry = Réessayer

volume_scroll_step = Pas de défilement du volume

# Radio Registries
radio_registry_url_placeholder = URL ou chemin local vers index.json
add_radio_registry = Ajouter un Registre de Radio
radio_default_registry = Registre par Défaut


radio_registry_empty_path = Le chemin du registre ne peut pas être vide
radio_registry_exists = Le registre existe déjà
radio_registry_import_failed = Échec de l"importation du registre : { $error }
radio_registry_enable_failed = Échec de l"activation du registre : { $error }
radio_registries_active = { $enabled_count }/{ $total } actifs
radio_registries_empty = Aucun registre configuré
offline_downloads = Téléchargements hors ligne
download_quality = Qualité du téléchargement
saving = Enregistrement...
discover_more_loading = Chargement…
discover_failed = Impossible de charger Découvrir : { $error }
discover_show_all = Tout voir
catalog_page_explore = Explorer
catalog_page_charts = Classements
catalog_page_moods = Ambiances et genres
catalog_page_podcasts = Podcasts
catalog_page_library_songs = Vos titres
catalog_page_library_albums = Vos albums
catalog_page_library_artists = Vos artistes
catalog_page_subscriptions = Abonnements
catalog_page_library_podcasts = Vos podcasts
catalog_page_uploads = Mises en ligne
catalog_page_history = Historique
search_filter_all = Tout
search_filter_songs = Titres
search_filter_videos = Vidéos
search_filter_community_playlists = Playlists de la communauté
search_filter_featured_playlists = Playlists à la une
search_filter_episodes = Épisodes
search_filter_profiles = Profils
discover_show_more = Afficher plus
discover_scroll_back = Défiler vers l’arrière
discover_scroll_forward = Défiler vers l’avant
catalog_page_empty = Rien ici pour l’instant
search_prompt = Cherchez des titres, albums, artistes et plus
search_showing_results_for = Résultats pour « { $query } »
catalog_like = J’aime
catalog_unlike = Retirer le J’aime
catalog_dislike = Je n’aime pas
catalog_undislike = Retirer le Je n’aime pas
catalog_save = Enregistrer dans la bibliothèque
catalog_unsave = Retirer de la bibliothèque
catalog_follow = Suivre
catalog_unfollow = Ne plus suivre
catalog_following = Abonné
catalog_remove_history = Retirer de l’historique
playlist_edit_details = Modifier les détails
playlist_description = Description
playlist_privacy = Qui peut la voir
playlist_privacy_public = Publique
playlist_privacy_unlisted = Non répertoriée
playlist_privacy_private = Privée
playlist_delete_title = Supprimer cette playlist ?
playlist_delete_body = « { $name } » sera supprimée définitivement de { $service }.
playlist_unsave_title = Retirer de votre bibliothèque ?
playlist_unsave_body = « { $name } » quitte votre bibliothèque { $service }. La playlist reste sur { $service }.
source_anon_playlists = Connectez-vous à YouTube Music pour voir vos playlists.
source_anon_favorites = Connectez-vous à YouTube Music pour voir vos titres aimés.
source_anon_discover = Connectez-vous à YouTube Music pour parcourir Découvrir.
artist_none_selected = Aucun artiste sélectionné

# Metadata Viewer / Editor
view_metadata = Voir les métadonnées
edit_metadata = Modifier les métadonnées
edit = Modifier
track_number = Piste n°
disc_number = Disque n°
disc_heading = Disque { $number }
duration = Durée
sample_rate = Fréquence d'échantillonnage
bitrate = Débit
musicbrainz_release = Album MusicBrainz
musicbrainz_recording = Enregistrement MusicBrainz
musicbrainz_track = Piste MusicBrainz
path = Chemin
add_photo = Ajouter une photo
change_photo = Changer la photo
remove_photo = Supprimer la photo
metadata_edit_warning = Les champs vides suppriment l'étiquette. Écrit directement dans le fichier — aucune annulation possible.

sort_by = Trier par
sort_then = puis
sort_none = Aucun tri
sort_add_criterion = Ajouter un critère
sort_remove = Enlever
sort_ascending = Croissant
sort_descending = Décroissant
sort_field_title = Titre
sort_field_artist = Artiste
sort_field_year = Année
sort_field_genre = Genre
sort_field_album = Album
sort_field_duration = Durée
sort_field_date_added = Date d'ajout
sort_field_name = Nom
sort_field_tracks = Pistes
sort_field_albums = Albums
view_grid = Vue grille
view_list = Vue liste
playback_browser = Navigateur de lecture Spotify
playback_browser_auto = Automatique
spotify_connect_device = Appareil de lecture Spotify
spotify_connect_device_other = Autre appareil
spotify_connect_device_this = Cette application
external_play_on = Lire sur
external_this_app = kopuz (cette application)
prefer_active_device = Playback device
prefer_active_device_help = When another device is already playing, take it over instead of starting playback here.
settings_managed_notice = Certains réglages proviennent d'un fichier de réglages que l'application ne peut pas modifier (en lecture seule ou géré par Nix/hjem) et ne peuvent pas être changés ici.
setting_managed_by_system = Géré par la configuration système
settings_dropin_notice = Certains réglages sont définis par des fichiers complémentaires et ne peuvent pas être modifiés ici.
settings_env_notice = Certains réglages sont définis par des variables d'environnement KOPUZ_CONFIG_* et ne peuvent pas être modifiés ici.
setting_locked_by_file = Défini par { $path }
setting_locked_by_dropin = Défini par le fichier complémentaire { $path }
setting_locked_by_env = Défini par { $var }

settings_interface = Interface
interface_general = General
interface_motion = Motion
interface_motion_description = How much the interface animates.
interface_motion_full = Full
interface_motion_reduced = Reduced
interface_motion_off = Off
interface_accent = Accent from artwork
interface_accent_description = Tint the player with colour taken from the current cover.
interface_accent_on = Everywhere
interface_accent_sheet = Now playing only
interface_accent_off = Off
interface_scale = Interface size
interface_scale_description = Scales text and controls across the whole app.

choose = Choose
daemon_unreachable = Can't reach the kopuz daemon
download_offline = Télécharger hors ligne
go_to_album = Aller à l'album
go_to_artist = Aller à l'artiste
new_playlist_name = New playlist name
playlist_name_required = Playlist name is required
remove_download = Supprimer le téléchargement
settings_connectivity = Connectivity
settings_customization = Appearance
settings_general = General
settings_library = Library
settings_metadata = Metadata
settings_offline = Offline Downloads
settings_player = Player
settings_tools = Tools
share = Share
share_copied = Copied link
share_no_page = Couldn't find a page for this track
explicit = Explicite

first_run_title = Welcome to Kopuz
first_run_subtitle = Point Kopuz at your music and it takes care of the rest.
first_run_step_source = Source
first_run_step_connect = Connect
first_run_step_scan = Scan
first_run_server = Media server
first_run_server_detail = Jellyfin, Subsonic, Navidrome or Nextcloud
first_run_streaming = Streaming service
first_run_streaming_detail = YouTube Music, SoundCloud, Apple Music or Spotify
first_run_scan = Scan my library
first_run_server_title = Choose a media server
first_run_server_hint = Pick a service and Kopuz will ask for what it needs.
first_run_streaming_title = Choose a streaming service
first_run_streaming_hint = Signing in opens your browser; Kopuz never sees your password.
first_run_no_services = No services available
first_run_sync_title = Building your library
first_run_sync_hint = This runs in the background. Tracks appear as they are found.
first_run_waiting_sign_in = Waiting for you to finish signing in…
first_run_progress = { $current } of { $total }
first_run_go_to_library = Go to library
first_run_back = Back
first_run_skip = Skip for now

play_pause = Play/Pause
next_track = Next Track
previous_track = Previous Track
seek_forward = Seek Forward
seek_backward = Seek Backward
volume_up = Volume Up
volume_down = Volume Down
toggle_mute = Mute
toggle_fullscreen = Fullscreen Player
command_palette = Command Palette
filter_view = Filter
go_back = Retour
go_forward = Forward
keyboard_shortcuts = Keyboard Shortcuts

settings_search = Search settings
settings_unavailable = Settings aren't available
settings_unavailable_hint = Kopuz hasn't read the daemon's configuration yet.
setting_saved = Saved
secret_saved = saved ••••
waiting_for_browser = Waiting for your browser…
signed_in = Signed in
metadata_number_invalid = Enter a number, or leave it empty to clear the tag.
metadata_batch_keep = Leave unchanged
metadata_batch_count =
    { $count ->
        [one] Editing { $count } track
       *[other] Editing { $count } tracks
    }
interface_font = Interface font
interface_font_description = The font family to draw the interface with.
interface_font_placeholder = Theme default
theme_description = The colour palette the whole app is drawn with.
auto_check_updates_description = Look for a new Kopuz release on startup.
minimize_to_tray_description = Closing the window keeps Kopuz running in the tray.
back_behavior_description = What the back button does partway through a track.
cover_art_background_description = Use the current cover as the window background.
cover_art_background_off = Off
cover_art_background_cover = Cover
cover_art_background_blurred = Blurred
custom_background_description = An image to use instead of the cover art background.
custom_font_description = A font file to load and draw the interface with.
reduce_animations_description = Cut transitions and motion across the app.
show_source_toggle_description = Put the source switcher in the chrome.
show_row_images_description = Draw cover thumbnails in track and album lists.
download_quality_description = The bitrate downloads are saved at, where the source offers a choice.
prefer_local_lyrics_description = Use .lrc files and embedded lyrics only; never ask a lyrics service.
enable_musixmatch_lyrics_description = Fall back to Musixmatch when a track has no local lyrics.
lyrics_offset_description = Shift the lyrics against the audio, in milliseconds.
lyrics_offset_auto_description = Shift by the engine's measured output latency, currently { $latency } ms.
crossfade_description = Overlap the end of one track with the start of the next.
volume_scroll_step_description = How far one scroll over the volume control moves it.
channel_mode_description = Remap the output channels, for one speaker or a broken headphone.
device_change_behavior_description = What playback does when the audio output device changes.
sample_rate_mode_description = Resample to the system rate, or follow each track's own.
equalizer_description = A gain curve applied to everything that plays.
enable_tracing_description = Write a performance trace for diagnosing slowness.

interface_nav = Navigation
interface_nav_position = Navigation bar
interface_nav_position_description = Where the destinations sit.
interface_nav_position_top = Across the top
interface_nav_position_left = Down the left
interface_nav_start = Opens on
interface_nav_start_description = The destination the app shows when it starts.
interface_panel = Playlists panel
interface_panel_shown = Show the panel
interface_panel_shown_description = A column of your playlists and folders, left of the content.
interface_panel_width = Panel width
interface_panel_width_description = How wide the panel opens; dragging its edge also sets this.
toggle_playlists_panel = Playlists panel

palette_placeholder = Search, or type > for commands
palette_mode_commands = Commands
palette_mode_artists = Artists
palette_mode_albums = Albums
palette_mode_playlists = Playlists
palette_current_track = Current track · { $title }
palette_recent_commands = Recent
palette_recent_searches = Recent searches
palette_commands = Commands
palette_navigate = Go to
palette_empty = Type to search, or > @ # : to narrow
palette_hint_move = Move
palette_hint_open = Open
palette_hint_actions = Actions
palette_hint_close = Close
interface_palette = Command palette
palette_history = Remember recent commands and searches
palette_history_description = An empty palette offers what was used last.
palette_recents = How many to remember
palette_recents_description = Beyond this, the oldest is forgotten.
palette_recents_count = { $count }
palette_current_track_first = Current track actions first
palette_current_track_first_description = While something is playing, its own verbs lead the list.

downloads = Downloads
notifications = Notifications
activity_idle = Nothing running
downloads_idle = No downloads yet
notifications_empty = No notifications yet
stop = Stop
stopping = Stopping…
starting = Starting…
clear = Clear
retry_failed = Retry failed
retry_now = Retry now
view = View
undo = Undo
quit = Quit
menu_hide = Hide { $app }
menu_hide_others = Hide Others
menu_show_all = Show All
menu_services = Services
menu_quit = Quit { $app }
menu_window = Window
menu_zoom = Zoom
menu_toggle_full_screen = Toggle Full Screen
redo = Redo
cut = Cut
paste = Paste
scanning_library = Scanning library…
network_offline_banner = Pas de connexion internet — passage en mode hors ligne
network_online_banner = De nouveau en ligne — passage en mode serveur
keep_server_mode = Rester en mode serveur
tray_show_hide = Afficher / Masquer Kopuz
tray_quit = Quitter Kopuz
tray_unavailable_title = Icône de la zone de notification indisponible
tray_unavailable_message = Zone de notification indisponible : la bibliothèque appindicator est introuvable. Installez libayatana-appindicator (Debian/Ubuntu/Arch) ou libappindicator-gtk3 (Fedora). Fermer la fenêtre quittera l'application au lieu de la réduire dans la zone de notification.
reconnecting_attempt = Reconnecting… (attempt { $attempt })
downloads_summary = { $done } downloaded, { $failed } failed
job_scan = Library scan
job_library_sync = Library sync
job_favorites_sync = Favorites sync
job_playlist_sync = Playlist sync
job_download = Downloading tracks
job_url_download = URL download
job_audio_rip = Disc rip
job = Background job
job_finished = { $job } finished
job_failed = { $job } failed
job_done = Finished
job_stopped = Stopped
removed_from_queue = Removed "{ $title }" from the queue
unfavorited_count = Removed { $count } from Favorites
track_removed_from_playlist = Removed from the playlist
interface_notifications = Notifications
interface_notifications_errors = Error toasts
interface_notifications_errors_description = Errors interrupt with a toast. They always land in the notification centre.
interface_notifications_job_description = Toast when this finishes, or only list it in the notification centre.

now_playing = Now playing
pause = Pause
playing_from = Playing from
queue_history = History
queue_now = Now playing
queue_next_up = Next up
clear_upcoming = Clear what's next
cleared_upcoming = Cleared { $count } from the queue
shuffle_upcoming = Shuffle what's next
save_queue_as_playlist = Save queue as a playlist
related = Related
related_soon = Related tracks are not here yet
playback_extras = Playback
sleep_timer = Sleep timer
playback_speed = Playback speed
interface_player = Player bar
interface_player_cover = Cover size
interface_player_cover_description = How large the cover is drawn in the bar.
interface_player_cover_small = Small
interface_player_cover_medium = Medium
interface_player_cover_large = Large
interface_player_shuffle = Show shuffle
interface_player_repeat = Show repeat
interface_player_volume = Show volume
interface_player_queue = Show the queue button
interface_player_lyrics = Show the lyrics button
interface_player_favorite = Show the favourite button
interface_sheet = Now playing
interface_sheet_tab = Opens on
interface_sheet_tab_description = The tab the now-playing sheet shows first.
interface_sheet_side = Panel side
interface_sheet_side_description = Which side of the sheet the queue and lyrics sit on.
interface_sheet_side_left = Left
interface_sheet_side_right = Right
interface_queue_style = Queue style
interface_queue_style_description = Sections folds away what has played and jumps back to the top on a new song. Classic is one list that keeps the playing song in view, like the old right bar.
interface_queue_style_sections = Sections
interface_queue_style_classic = Classic
interface_sheet_backdrop = Backdrop
interface_sheet_backdrop_description = What is behind the sheet.
interface_sheet_backdrop_artwork = The cover
interface_sheet_backdrop_palette = Colour from the cover
interface_sheet_backdrop_solid = Nothing

discover_search_hint = Search everything
discover_clear_search = Clear search
discover_results_for = Results for "{ $query }"
discover_no_results_hint = Try fewer words, or switch source and look again.
discover_show_less = Show less
discover_genre_count =
    { $count ->
        [one] { $count } genre
       *[other] { $count } genres
    }
discover_read_failed = Couldn't load your library: { $error }
palette_all_results = All results for "{ $query }" in Discover

library_count =
    { $count ->
        [one] { $count } item
       *[other] { $count } items
    }
library_filter_placeholder = Filter the library…
library_chip_downloaded = Downloaded
library_chip_years = Years
library_chip_folder = Folder
library_chip_any = Any
library_group_by = Group by
library_group_none = None
library_group_unknown = Unknown
library_columns = Columns
library_column_plays = Plays
library_column_format = Format
library_density = Row height
library_density_description = How much room each row is given.
library_density_compact = Compact
library_density_comfortable = Comfortable
library_tile_size = Tile size
library_tile_size_description = How big the tiles in a grid are drawn.
library_tile_small = Small
library_tile_medium = Medium
library_tile_large = Large
library_default_view = Grids open as
library_default_view_description = Whether albums, artists and playlists start as tiles or as rows.
library_artist_tracks =
    { $count ->
        [one] { $count } track
       *[other] { $count } tracks
    }
library_selected = { $count } selected
library_whole_selection = Everything that matches
library_read_failed = Could not read the library
library_no_matches = Nothing matches those filters
library_no_matches_detail = Take a chip off, or change what you typed.
library_empty_signed_out = Sign in to see this library
library_sign_in = Sign in
library_empty_no_folder = No music folder yet
library_empty_no_folder_detail = Point Kopuz at a folder and it will scan what is in it.
library_add_folder = Add a music folder
library_empty_not_synced = Nothing here yet
library_empty_not_synced_detail = Pull the library from the server to fill it.
interface_library = Library
playlist_empty = This playlist is empty
playlist_empty_detail = Add tracks to it from anywhere in the library.

home_continue = Continue
home_queue_left = { $count } left
recently_played = Recently played

shortcuts_playback = Playback
shortcuts_queue = Queue
shortcuts_app = App
shortcuts_editor_hint = Choose a shortcut to change it, then press the keys you want. Escape cancels.
shortcuts_recording = Press a key…
shortcuts_unbound = Not bound
shortcuts_conflict = { $key } is already { $command }.
theme_unsaved_title = Unsaved theme changes
theme_unsaved_body = Switching themes now throws away the colours you changed.
discard_changes = Discard

downloader_add_url = Add a URL…
downloader_url_title = Download from a URL
downloader_history = URL downloads

interface_skin = Skin
interface_skin_description = The shape of the interface: how round things are, how tall a row sits.

home_plays =
    { $count ->
        [one] { $count } play
       *[other] { $count } plays
    }

interface_ambient = Ambient light
interface_ambient_description = Let the current cover cast its colour across the window.
interface_ambient_glow = Glow
interface_ambient_full = Whole window

daemon_older = The background service is older than this app. Update it or restart it.
daemon_newer = Kopuz was updated while it was open. Restart it to finish updating.

sign_in_with_webview = Sign in within Kopuz
webview_sign_in_help = Sign-in opens inside Kopuz. Close the sign-in window to cancel.

settings_group_app = Application
settings_group_playback = Lecture
settings_group_services = Services
settings_group_theme = Thème
settings_group_background = Arrière-plan
settings_group_interface = Interface
settings_group_output = Sortie
settings_group_loudness = Volume
browse = Parcourir

clippsly_help = Create an App Password at clippsly.app, then sign in with your Clippsly username and that App Password. Storage limits are set by your Clippsly account.
cloud_storage_usage = Cloud storage: { $used } of { $total } used
upload_music = Upload music
uploading_music = Uploading { $name }…
music_uploaded =
    { $count ->
        [one] Uploaded { $count } file.
       *[other] Uploaded { $count } files.
    }
music_upload_too_large = Audio files must be no larger than 500 MiB.
cloud_storage_loading = Loading storage usage…

smb_url_placeholder = smb://nas/Music
smb_help = Connect to an SMB 2/3 share or subfolder, then sign in with your NAS account. Domain accounts can use DOMAIN\username. Files are read without mounting the share.
invalid_smb_url = Enter smb://host/share or smb://host/share/folder, without credentials in the URL.

browser_sessions_heading = Déjà connecté
browser_sessions_checking = Vérification de vos navigateurs…
browser_session_name = { $browser } · { $profile }
browser_session_use = Utiliser
browser_session_importing = Connexion…
browser_session_help = Kopuz copie la session du profil de navigateur choisi. Le navigateur peut rester ouvert.

playback_error_session_expired = Votre connexion à { $source } a expiré. Reconnectez-vous pour continuer la lecture.
playback_error_challenged = { $source } veut vérifier que vous n'êtes pas un robot. Patientez quelques minutes et réessayez.
playback_error_refused = { $source } a refusé de lire ce morceau. Votre connexion est correcte : le problème vient de { $source } et se résout généralement tout seul.
playback_error_unreachable = Impossible de joindre { $source }. Vérifiez votre connexion et réessayez.
playback_error_unavailable = { $source } ne peut pas lire ce morceau. Il a peut-être été supprimé, rendu privé ou bloqué dans votre région.
playback_sign_in_again = Se reconnecter
playback_try_again = Réessayer
playback_skip = Passer le morceau
playback_copy_details = Copier les détails
playback_details_copied = Détails copiés
source_signed_out = ● Déconnecté
source_not_responding = ● Ne répond pas
connection_test = Tester la connexion
connection_testing = Test en cours…
connection_test_failed = Impossible de lancer le test : { $error }
connection_copy_report = Copier le rapport
connection_report_copied = Rapport copié
connection_step_reach = Service joignable
connection_step_session = Connexion acceptée
connection_step_signed_in_playback = Lecture avec votre compte
connection_step_anonymous_playback = Lecture sans compte
connection_step_playback_token = Jeton de lecture
connection_step_passed = Réussi
connection_step_failed = Échec
connection_step_skipped = Ignoré

sync_interval = Sync and rescan every
sync_interval_never = Never
sync_interval_minutes = { $count } min
sync_interval_hours = { $count } h
sync_interval_days = { $count } d

error_setting_locked = A managed settings file pins this setting, so it cannot be changed here.
error_connect_not_offered = This integration does not connect that way.
error_api_key_missing = Set the API key and secret first.
error_sign_in_not_started = The service did not start the sign-in. Check the API key and your connection.
error_browser_not_opened = Could not open a browser for the sign-in.
error_sign_in_timed_out = The sign-in was not approved in time.
error_browser_profile_gone = That browser profile is no longer there.
error_browser_cookies_unreadable = Could not read the cookies of that browser profile.
error_browser_profile_signed_out = That browser profile is not signed in to the service. Sign in there, or paste your token instead.
error_browser_profile_no_token = The service did not show a token for that profile. Paste it instead.
error_integration_unreachable = Could not reach the service. Check your connection.
error_credential_rejected = The service does not accept that token. Paste your token instead.
error_default_source_undeletable = The default folder source cannot be deleted.
error_empty_source_folder = A source folder cannot be empty.
error_code_invalid_input = Not accepted: { $detail }
error_code_not_found = That is no longer there.
error_code_conflict = Already in use or in progress: { $detail }
error_code_source_auth_expired = The source did not accept the sign-in. Sign in again in Settings.
error_code_source_unreachable = The source could not be reached. Check your connection.
error_code_unsupported = Not supported here: { $detail }
error_code_internal = Something went wrong: { $detail }
error_code_source_challenged = The source wants to check that you're not a bot. Wait a few minutes and try again.
error_code_source_refused = The source refused the request. This is on its side and usually clears up on its own.
error_code_item_unavailable = The source can't play this. It may have been removed, made private or blocked in your region.
copy_image = Copy image
save_image = Save image…
image_copied = Image copied
image_saved = Image saved
image_copy_failed = Couldn't copy the image: { $error }
image_save_failed = Couldn't save the image: { $error }
for_you = For you
service_summary_folders = Music files on this computer or a mounted drive
service_summary_jellyfin = Music from your own Jellyfin server
service_summary_subsonic = Navidrome, Airsonic or a Subsonic server of your own
service_summary_custom = Any other server with a Subsonic-compatible API, at the address you enter
service_summary_ytmusic = YouTube Music's catalog, plus your library and likes when you sign in
service_summary_soundcloud = Tracks, playlists and likes from your SoundCloud account
service_summary_applemusic = Apple Music's catalog and your library; playback needs a subscription
service_summary_spotify = Your Spotify library, played through Spotify in a browser; needs Premium
service_summary_nextcloud = Music files stored in your Nextcloud
service_summary_clippsly = Music you upload to your Clippsly cloud locker, streamed back from there
service_summary_smb = Music files on a network share (SMB), read without mounting it
onboarding_welcome_blurb = Kopuz plays the music on this computer, on your own server and on your streaming services, all from one place.
onboarding_desktop_library_note = If you use the Kopuz desktop app, your library is already here
onboarding_privacy_title = Privacy
onboarding_privacy_discord = Discord presence shows the song you're playing on your Discord profile.
onboarding_privacy_updates = Update checks ask GitHub for the newest Kopuz release when the app starts.
onboarding_privacy_watch_history = YouTube watch history adds the songs you play here to your YouTube history, which shapes its recommendations.
onboarding_ready_summary =
    { $tracks ->
        [one] { $tracks } track
       *[other] { $tracks } tracks
    }, { $albums ->
        [one] { $albums } album
       *[other] { $albums } albums
    }
onboarding_start_listening = Start listening
onboarding_run_again = Run setup again
onboarding_probe_found =
    { $count ->
        [one] Found { $count } track
       *[other] Found { $count } tracks
    }
onboarding_probe_empty = No music found in this folder
onboarding_probe_unreachable = Can't reach { $url }
onboarding_step = Step { $current } of { $total }
edit_url = Edit URL
continue = Continue
onboarding_group_local = On this computer
tips_title = A few things to know
tips_search = searches everything and runs commands
tips_now_playing = Click the cover in the player bar to open Now Playing
tips_source_switcher = Switch between your sources from the menu in the top bar
daemon_down_title = Kopuz needs its background service
daemon_not_answering = A Kopuz daemon is already running but not answering. Quit it, then retry.
retry = Retry
connectivity_check_urls = Connectivity checks
connectivity_check_urls_description = Addresses asked when a source stops answering, to tell a dropped connection from a server that is down. Leave empty to skip the check.
error_network_down = You're offline. Until the connection is back, only music on this computer or your local network can play.
offline_mode = Offline mode
offline_mode_on = Offline mode is on. Only downloads and cached songs play.
offline_mode_off = Offline mode is off.
offline_mark = Offline
cached_mark = Cached
offline_cache = Offline cache
offline_cache_enabled = Keep songs you play for offline listening
offline_cache_max_size = Maximum size
offline_cache_used = { $used } of { $max } used
offline_cache_clear = Clear cache
offline_cache_description = Songs you stream from across the internet are kept on this computer so they play without a connection, and the next song is fetched ahead. Once the cache is full, the songs played longest ago go first. Downloads are kept apart and never removed. Turning this off keeps what is cached.
back_online_switch_to = Back online, switch to { $source }?
onboarding_offline_cache = Keep songs you play for offline listening (up to { $size })
error_offline_mode = Offline mode is on, and this song has no downloaded or cached copy to play.
offline_mode_description = Only downloads and cached songs play, and nothing reaches the internet until you turn it off.
offline_mode_turn_off = Turn off offline mode
network_moved_offline = No internet connection. Moved to { $source }, which works without it.
back_online = Back online.
back_online_still_offline = Back online. Offline mode stays on until you turn it off.
source_needs_internet = Needs the internet
discover_your_mixes = Your mixes
discover_never_played = Never played
discover_forgotten_favorites = Forgotten favorites
library_empty_offline = Nothing is available offline yet
library_empty_offline_detail = Download songs, or play them while online, to keep them for offline listening.

# Audio CDs
cd_rip_flac = Rip to FLAC
cd_rip_help = Save all tracks as lossless FLAC files. Playback stops while ripping. Existing files are kept. Requires ffmpeg.
cd_output_directory = Output folder (absolute path on the computer running Kopuz)
cd_rip_finished = Ripping finished. You can add the output folder to a local library.
cd_rip_cancelled = Ripping cancelled. Completed tracks have been kept.

removable_found = Removable media found
removable_listen_help = Switch to this disc and start listening? It stays available while inserted. Removing it returns you to your previous source with playback stopped.
removable_later = Not now
removable_listen = Listen
removable_temporary = Temporary — until ejected
