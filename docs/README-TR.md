<!--markdownlint-disable MD013 MD033 MD041 -->
<div align="center">
  <img src="../crates/kopuz/assets/banner.png" alt="Kopuz Logo" height="300"/>
  <h1>Kopuz</h1>
  <p>
    Kopuz; Rust ve Dioxus framework'ü ile oluşturulmuş, modern ve hafif bir müzik çalar
    uygulamasıdır. Yerel müzik koleksiyonunuzu yönetmeniz ve keyfini çıkarmanız için
    temiz ve responsive bir arayüz sunar.
  </p>
  <a href="https://discord.gg/K6Bmzw2E4M">
    <img src="https://img.shields.io/badge/Discord-5865F2?style=flat&logo=discord&logoColor=white" alt="Discord">
  </a>
  <img src="https://img.shields.io/badge/Rust-000000?style=flat&logo=rust&logoColor=white" alt="Rust">
  <img src="https://github.com/Kopuz-org/kopuz/actions/workflows/build.yml/badge.svg" alt="Build">
  <img src="https://github.com/user-attachments/assets/2b12ec40-2fcb-45e9-969e-ef99b4654957" alt="Kopuz">

<br/>
  <br/>
  <p>
    <a href="../README.md">English</a> | <b>Türkçe</b>
  </p>
</div>

## About the Name

_Kopuz_, eski bir Türk telli çalgısıdır ve genellikle birçok Orta Asya
lavtasının atası olarak kabul edilir. Geleneksel olarak bards ve şamanlar
tarafından kullanılırdı.

Kırgız _komuz_'u aynı enstrüman olmasa da muhtemelen _kopuz_'un bir soyundan
gelmektedir. Kazak _kobyz_'ı da mızrapla çalınmak yerine yayla çalınmasına
rağmen ilişkilidir. Buna karşılık, Tuvan/Yakut _xomus_ (jaw harp) ise benzer
isme rağmen tamamen ilgisizdir.

Türk efsanelerinde _kopuz_, mitolojik olmaktan ziyade efsanevi bir ozan olan
Dede Korkut ile ilişkilendirilir.

## Overview

Kopuz; ses dosyaları için yerel dizinlerinizi taramanıza, Jellyfin veya Subsonic
(Navidrome vb.) sunucunuzdan stream etmenize veya **YouTube Music**'i bir
streaming backend'i olarak bağlamanıza olanak tanır — her şeyi otomatik olarak
göz atılabilir bir kütüphanede düzenler. Sanatçılara, albümlere, türlere göre
gezinebilir veya özel playlist'lerinizi keşfedebilirsiniz. Uygulama, Rust'ın
gücünü kullanarak performans ve desktop integration için oluşturulmuştur.

## Features

[jellyfin-plugin-listenbrainz]: https://github.com/lyarenei/jellyfin-plugin-listenbrainz

- **Theming**: Görsel görünümü özelleştirmek için dinamik theming desteği
  içerir. Ayrıca tam renk değişkeni kontrolü ile sıfırdan kendi custom
  theme'inizi oluşturabilirsiniz.
- **Native Integration**: Linux (MPRIS), macOS (Now Playing / Remote Command
  Center) ve Windows (System Media Transport Controls) üzerindeki sistem medya
  kontrolleri ile entegre olur.
- **Discord RPC**: Gömülü RPC dahildir!!!
- **Multiple Backends**: Stream edin (Navidrome works great), YouTube Music'i
  bağlayın veya sadece yerel bir klasörü gösterin. İstediğiniz gibi karıştırıp
  eşleştirin.
- **YouTube Music**: Spotify tarzı bir **Discover** sayfası (önerilen şarkılar,
  playlist'ler, albümler, sanatçılar ve ruh halleri), zengin **artist profiles**
  (banner, en popüler şarkılar, albümler, single'lar, benzer sanatçılar),
  albüm/playlist tarama ve **mix radyo** ("start radio" from any track) içeren
  tam bir streaming backend'i. Kütüphaneniz, Beğenilen Müzikleriniz ve
  playlist'leriniz için hesabınızla giriş yapın — veya herkese açık parçaları
  aramak, göz atmak ve oynatmak için **anonymously** (no sign-in) çalıştırın.
  Bkz. [YouTube Music Setup](#youtube-music-setup).
- **Lyrics Support**: Müziğinizi takip etmek için auto-scrolling özelliğiyle
  birlikte gerçek zamanlı senkronize ve düz şarkı sözlerinin keyfini çıkarın.
- **Favorites**: Parçaları yerel olarak yıldızlayın veya favorileri
  Jellyfin/Subsonic sunucunuzla senkronize edin.
- **Playlists**: Kendi playlist'lerinizi oluşturun ve yönetin, tek tek parçaları
  veya tüm albümleri tek seferde ekleyin ve playlist'leri sunucunuzla senkronize
  edin.
- **Genre Browsing**: Hem yerel hem de sunucu müzikleri için kütüphanenizi türe
  göre tarayın.
- **Search**: Sanatçılar, albümler ve parçalar arasında gerçek zamanlı
  sonuçlarla arama yapın.
- **Listening Logs**: En çok neyi dinlediğinizi görebilmeniz için çalma
  sayılarını yerel olarak takip eder.
- **Scrobbling**: ListenBrainz'e scrobble edin. Birden fazla istemci
  kullanıyorsanız, Jellyfin kullanıcıları için [jellyfin-plugin-listenbrainz]
  önerilir.
- **Language Support**: UI dili İngilizce, Rusça, Almanca, Fransızca,
  İspanyolca, Türkçe, Ukraynaca, Lehçe, Arapça, Yunanca, İbranice, Macarca,
  Endonezce, Japonca, Korece, Romence, Brezilya Portekizcesi, Toki Pona ve
  Basitleştirilmiş Çince dillerinde mevcuttur ve yeni diller eklemek için
  kolaylaştırılmış bir deneyim sunar.
- **High Performance**: Yoğun background processing ve optimize edilmiş bir
  kütüphane tarayıcısı, uygulamanın anında açılmasını, sorunsuz çalışmasını ve
  daha önce indekslenmiş dosyaları hızla atlamasını sağlar.
- **Auto-Cleanup**: Yeniden tarama yaparken eksik veya silinmiş parçaları
  kütüphanenizden otomatik olarak kaldırır.
- **Smooth Navigation**: Farklı görünümlere ve sayfalara göz atarken scroll
  pozisyonlarının düzgün bir şekilde sıfırlandığı cilalı bir arayüzün keyfini
  çıkarın.
- **Reduce Animations**: Daha sakin bir UI tercih ediyorsanız hareket
  efektlerini hafifletmek için erişilebilirlik ayarı.
- **Equalizer**: Sesinize ince ayar yapmak için hazır ayarlar (presets) ve özel
  ayarlara sahip dahili 5 bantlı ekolayzır.
- **Crossfade**: Native masaüstü derlemelerinde şarkılar arasında daha yumuşak
  otomatik oynatmak için parça geçişlerini harmanlayın. Tarayıcı oynatımı şu
  anda normal parça geçişini kullanır.
- **Channel Mode**: Switch between `Stereo`, `Mono`, `Left only`, `Right only`,
  ve `Swap L/R` çıkış modları.
- **yt-dlp Integration**: yt-dlp aracılığıyla doğrudan YouTube ve desteklenen
  diğer sitelerden ses indirin. Çıkış formatınızı seçin (Best Audio, MP3, FLAC,
  WAV veya MP4 video). yt-dlp kayıpsız bir kaynaktan kod çözmek yerine kayıplı
  sesi yeniden paketlediği (remux) için FLAC önerilmez. SponsorBlock, bölüm
  bölme (chapter splitting), çerezler (cookies), hız sınırlama (rate limiting)
  ve daha fazlasını destekler. Sisteminizde `yt-dlp` kurulu olmasını gerektirir.
- **Metadata Settings**: Ayarlar'daki özel bir Metadata bölümü, sanatçı
  resimlerinin nasıl alınacağını kontrol etmenizi sağlar. **Album Cover**
  (sanatçı fotoğrafı olarak ilk albüm kapağını kullanır, varsayılan) veya
  **Artist Photo** (gerçek sanatçı resimlerini doğrudan Jellyfin veya Subsonic
  sunucunuzdan çeker) seçeneklerinden birini seçin. Artist Photo moduna
  geçildiğinde, Sanatçılar sayfasını açtığınız anda resimler arka planda
  sunucudan çekilir. Sunucunuzda bir sanatçının özel fotoğrafı yoksa, hiçbir
  şeyin boş görünmemesi için fallback olarak ilk albüm kapağı kullanılır.

## Installation

### NixOS / Nix

**Run directly without installing:**

```bash
nix run github:temidaradev/kopuz
```

**Install to your profile:**

```bash
nix profile add github:temidaradev/kopuz
```

**On NixOS, using the flake:**

> [!TIP]
> Bu yöntem, Kopuz'u simgesi ve `.desktop` girişi olan düzgün bir sistem
> uygulaması olarak kurduğu için `nix profile` yerine önerilir.

Kopuz'u `flake.nix` girdilerinize (inputs) ekleyin:

```nix
{
  inputs.kopuz.url = "github:temidaradev/kopuz";
}
```

Ardından bunu sistem yapılandırmanıza aktarın ve derlemek yerine önceden
derlenmiş binary dosyasını indirmesi için Cachix substituter'ını ekleyin:

```nix
{
  nix.settings = {
    substituters      = ["https://kopuz.cachix.org" ];
    trusted-public-keys = ["kopuz.cachix.org-1:J2X3AnAYhKTJW5S3aCLoA1ckonQXVNZMQvhZA0YAufw="];
  };
}
```

Ardından paketi kurun:

```nix
{pkgs, kopuz, ...}: let
  kopuzPkg = kopuz.packages.${pkgs.stdenv.hostPlatform.system}.default

in {
  environment.systemPackages = [kopuzPkg];
}
```

### AUR (Arch Linux)

Tercih ettiğiniz yardımcıyı kullanarak AUR'dan yükleyin:

```bash
yay -S kopuz
# veya
paru -S kopuz
```

> **Note:** Öncelikle Dioxus 0.7.x sürümüyle eşleşen `dioxus-cli` kurulu
> olmalıdır:
>
> ```bash
> cargo install dioxus-cli --version "^0.7"
> ```

### Flatpak (Recommended)

Kopuz yakında Flathub'da mevcut olacaktır. Kaynak manifest'inden (source
manifest) yüklemek için:

```bash
git clone https://github.com/temidaradev/kopuz
cd kopuz
flatpak-builder --user --install --force-clean build-dir packaging/flatpak/com.temidaradev.kopuz.json
flatpak run com.temidaradev.kopuz
```

Dosyaya tıklayıp bir uygulama sağlayıcı ile (örneğin KDE Discover) de
açabilirsiniz.

### AppImage

> [!IMPORTANT]
> AppImage, sisteminizde `webkit2gtk-4.1` ve `gtk3` kurulu olmasını gerektirir.
> Bu bağımlılıklar pakete dahil edilmemiştir.
>
> Modern bir masaüstü ortamına sahip çoğu dağıtımda bunlar zaten mevcuttur.
> Henüz kurulu değillerse bunları manuel olarak yüklemeniz gerekecektir.

Arch tabanlı dağıtımlarda, AppImage bir `WebKitNetworkProcess` hatasıyla
çökerse, uygulamayı şununla çalıştırın:

```bash
LD_LIBRARY_PATH=/usr/lib ./kopuz_*.AppImage
```

Veya bir kez sembolik bağlar (symlinks) oluşturun (sudo gerektirir):

```bash
sudo mkdir -p /usr/libexec/webkit2gtk-4.1
sudo ln -s /usr/lib/webkit2gtk-4.1/WebKitNetworkProcess /usr/libexec/webkit2gtk-4.1/
sudo ln -s /usr/lib/webkit2gtk-4.1/WebKitWebProcess /usr/libexec/webkit2gtk-4.1/
sudo ln -s /usr/lib/webkit2gtk-4.1/WebKitGPUProcess /usr/libexec/webkit2gtk-4.1/
```

### Build from Source

#### Dependencies

**Using Nix**

> [!TIP]
> Nix, Kopuz'un birincil geliştirme aracıdır ve sistemler arasında tutarlı, saf
> ve yeniden üretilebilir (reproducible) bir ortamda derleme bağımlılıklarını
> elde etmek için önerilen yöntemdir.

```bash
# Using Nix3 CLI
nix develop
```

Eğer bir Direnv kullanıcısıysanız, sağlanan `.envrc` dosyasını kullanın:

```bash
# Using Direnv
direnv allow
```

Geliştirme ortamı içinde kendi usershell'inizi kullanmaya devam etmek
istiyorsanız Direnv önerilir.

**Arch Linux Based Systems**

```bash
sudo pacman -S rust cargo dioxus-cli base-devel cmake pkgconf opus alsa-lib xdotool webkit2gtk-4.1 gtk3 libsoup3 openssl
```

**Debian Based Systems**

```bash
sudo apt install rustc cargo build-essential cmake pkg-config libopus-dev libasound2-dev libxdo-dev libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libssl-dev
cargo install dioxus-cli
```

**Fedora Based Systems**

```bash
sudo dnf groupinstall "Development Tools" "Development Libraries"
sudo dnf install rust cargo cmake pkgconf-pkg-config opus-devel alsa-lib-devel libxdo-devel webkit2gtk4.1-devel gtk3-devel libsoup3-devel openssl-devel
cargo install --locked dioxus-cli
```

**openSUSE Based Systems**

```bash
sudo zypper install rust cargo cmake pkg-config libopus-devel alsa-devel xdotool webkit2gtk3-soup2-devel gtk3-devel libsoup3-devel libopenssl-devel
cargo install --locked dioxus-cli
```

#### Developing Kopuz

```bash
# Clone the repository
$ git clone https://github.com/Kopuz-org/kopuz

# Move to the cloned directory
cd kopuz

# Install npm dependencies
npm install

# Serve project with Dioxus CLI
dx serve --package kopuz
```

### macOS

**Quarantine note:** Bunun yerine bir `.dmg` indirdiyseniz, macOS bunu
engelleyebilir. Karantina bayrağını temizlemek için bir kez çalıştırın:

```bash
xattr -d com.apple.quarantine /Applications/Kopuz.app
```

### Where does Kopuz keep its files?

**macOS** üzerinde her şey Library klasörlerinizin altındadır:

- `~/Library/Application Support/com.temidaradev.kopuz/config.json` -
  ayarlarınız
- `~/Library/Caches/com.temidaradev.kopuz/library.json` - taranan kütüphane
- `~/Library/Caches/com.temidaradev.kopuz/playlists.json` - playlist'leriniz
- `~/Library/Caches/com.temidaradev.kopuz/covers/` - cached albüm kapakları
- `~/Library/Caches/com.temidaradev.kopuz/offline_tracks/` - indirilen parçalar

**Linux** üzerinde beklendiği gibi XDG spec'ini takip eder:

- `~/.config/kopuz/config.json` - ayarlarınız
- `~/.cache/kopuz/library.json` - taranan kütüphane
- `~/.cache/kopuz/playlists.json` - playlist'leriniz
- `~/.cache/kopuz/covers/` - cached albüm kapakları
- `~/.cache/kopuz/offline_tracks/` - indirilen parçalar

**Windows** üzerinde AppData klasörünüzü kullanır:

- `%APPDATA%\temidaradev\kopuz\config\config.json` - ayarlarınız
- `%LOCALAPPDATA%\temidaradev\kopuz\cache\library.json` - taranan kütüphane
- `%LOCALAPPDATA%\temidaradev\kopuz\cache\playlists.json` - playlist'leriniz
- `%LOCALAPPDATA%\temidaradev\kopuz\cache\covers\` - cached albüm kapakları
- `%LOCALAPPDATA%\temidaradev\kopuz\cache\offline_tracks\` - indirilen parçalar

Kapaklar görünmüyorsa veya kütüphane hatalı görünüyorsa, cache klasörünü silip
rescan düğmesine basmanız yeterlidir.

## YouTube Music Setup

Kopuz, YouTube Music'i bir streaming backend'i olarak kullanabilir. Şuradan
ekleyin: **Settings → Media servers → Add → YouTube Music**.

### Prerequisite: rustypipe-botguard

Oynatma (hem oturum açılmış hem de anonim modlarda), YouTube'un stream URL'leri
için gerektirdiği PO token'ı üretmek üzere
[`rustypipe-botguard`](https://crates.io/crates/rustypipe-botguard) yardımcısına
ihtiyaç duyar. Bunu bir kez yükleyin:

```bash
cargo install rustypipe-botguard --version 0.1.2
```

Sunucu ekleme iletişim kutusunda, bunun `PATH` üzerinde olduğunu doğrulamak için
bir **Check rustypipe-botguard** düğmesi bulunur. Bu olmadan parçalar resolve
edilir ancak oynatılamaz.

### Choosing a mode

Kurulum iletişim kutusu iki yöntem sunar:

- **Sign in with a browser** — Kopuz, Google giriş sayfasını **isolated browser
  profile** (a fresh, separate session; your normal browsing is never touched)
  açar, giriş yapmanızı bekler ve oturum çerezlerini çıkarır. Hangi yüklü
  Chromium ailesi tarayıcının (Chrome, Chromium, Brave, Edge veya Vivaldi)
  kullanılacağını seçin. Bu, **library, Liked Music, playlists, and followed
  artists** kilidini açar.

- **Continue without signing in (anonymous)** — giriş yok, çerez yok. **Göz
  atabilir, arama yapabilir, sanatçı/albüm/playlist sayfalarını açabilir, mix
  radyoyu başlatabilir ve herkese açık parçaları oynatabilirsiniz**. Liked
  Music, library playlists, ve takip etme/beğenme devre dışı bırakılır (bu
  görünümler bir "sign in to enable" uyarısı gösterir). Music Premium-only
  parçalar anonim olarak oynatılamaz.

> [!NOTE]
> **Windows** üzerinde browser sign-in şu anda devre dışıdır — Google hesapları
> sayfası isolated profile içinde boş olarak yüklenir. Windows kullanıcıları
> otomatik olarak anonim modu alır. Giriş yapma Linux ve macOS üzerinde
> çalışmaktadır. (`crates/server/src/ytmusic/isolated_profile.rs` dosyasında
> `TODO(windows-signin)` olarak izlenmektedir.)

### Premium tracks

Music Premium kilitli parçalar, birincil yol `UNPLAYABLE` döndürdüğünde yerel
bir [`yt-dlp`](https://github.com/yt-dlp/yt-dlp) resolve işlemine fallback
yapar, bu nedenle `yt-dlp`'nin kurulu olması bunlar için yardımcı olur. Anonim
mod, Premium-only içerikleri hiçbir şekilde oynatamaz.

## Logs & Debugging

Kopuz, [`tracing`](https://docs.rs/tracing) aracılığıyla log tutar. Bunun büyük
bir kısmına uygulamanın kendisinden erişilebilir — **Settings → Logs** bölümünde
**Open logs folder**, **Export logs** ve **Enable Performance Tracing** geçişi
bulunur — böylece kullanıcıların yararlı bir rapor göndermek için asla bir
terminale ihtiyacı olmaz.

### Where the files live

Tüm dosyalar log dizininde yer alır (**Open logs folder** düğmesi doğrudan
buraya gider):

- Linux: `~/.cache/kopuz/logs/`
- macOS: `~/Library/Caches/com.temidaradev.kopuz/logs/`
- Windows: `%LOCALAPPDATA%\temidaradev\kopuz\cache\logs\`

| File                    | What it is                                                                                                       |
| ----------------------- | ---------------------------------------------------------------------------------------------------------------- |
| `latest.log`            | Mevcut oturum. Span zamanlaması + olaylar; canlı log.                                                            |
| `kopuz-<timestamp>.log` | Önceki oturumlar, başlangıçta arşivlenir (son 10 tanesi tutulur). Yeniden başlatma asla önceki çalışmayı silmez. |
| `crash-<timestamp>.txt` | **only on a crash** (Rust paniği) yazılır: mesaj, backtrace, son log kuyruğu, uygulama/OS sürümü.                |
| `kopuz-trace.json`      | Performance trace — yalnızca tracing etkinleştirildiğinde (aşağıya bakın). Her çalıştırmada üzerine yazılır.     |

Zaman damgaları UTC `YYYY-MM-DD_HH-MM-SS` formatındadır, böylece dosyalar
kronolojik olarak sıralanır.

### Triage cheat-sheet

**App crashed →** otomatik olarak bir `crash-<timestamp>.txt` oluşturulur.
Kullanıcıdan **Settings → Logs → Export logs** (en yeni çökme raporu ile
`latest.log` dosyasını tek bir dosyada birleştirir) yapmasını veya **Open logs
folder** diyerek en yeni `crash-*.txt` dosyasını almasını isteyin.

**Performance issue (freeze / slow load / stutter) →** kullanıcıdan şunları
yapmasını isteyin:

1. **Settings → Logs → enable "Performance Tracing"**, ardından uygulamayı
   **yeniden başlatın** (bu geçiş bununla ilgili uyarır — trace kaydedici
   başlangıçta bir kez kurulur).
2. Yavaş eylemi yeniden gerçekleştirin.
3. **Uygulamadan çıkın** (bu, trace kaydını temiz bir şekilde kaydeder).
4. **Settings → Logs → Open logs folder** ve `kopuz-trace.json` dosyasını (veya
   **Export logs** dosyasını) gönderin.

Trace dosyasını [speedscope.app](https://speedscope.app) veya
[ui.perfetto.dev](https://ui.perfetto.dev) adresinde açın. Kritik yollar
(YouTube stream çözümleme, göz atma/arama/sayfalandırma, mix radyo, kütüphane
taraması, indirmeler, oynatma geçişleri, bileşen başına render'lar)
adlandırılmış span'ler olarak izlenir ve çalışan iş parçacığı (worker-thread)
işleri onu başlatan eylemin altında yuvalanır, böylece trace zamanın tam olarak
nereye gittiğini gösterir. İşiniz bittiğinde bunu tekrar kapatın — uzun
oturumlarda ek yük getirir ve trace dosyasını büyütür.

### Power-user env vars

Terminal çalıştırmaları için log **verbosity**'si env var'lar ile kontrol
edilir:

```bash
# Bir oturum için ayrıntılı (debug düzeyinde) loglar
KOPUZ_DEBUG=1 kopuz

# İnce ayarlı, modül başına (KOPUZ_DEBUG değerini geçersiz kılar); standart tracing yönerge sözdizimi
KOPUZ_LOG="server::ytmusic=trace,kopuz=debug" kopuz

# Derin render ağacı profilleme: Dioxus'un bileşen başına kendi render/diff span'leri
# (önce Settings'ten trace geçişini etkinleştirin; bu sadece neyin kaydedildiğini kontrol eder)
KOPUZ_LOG="info,dioxus_core=trace" kopuz
```

`RUST_LOG` da çalışır; `KOPUZ_LOG` önceliklidir.

**Performance trace** yalnızca **Settings → Logs → Enable Performance Tracing**
(ardından yeniden başlat) aracılığıyla etkinleştirilir — bunun için bir env var
yoktur; UI tek doğruluk kaynağıdır. Varsayılan olarak kapalıdır → sıfır ek yük.

> Debug derlemeleri, çökme raporu yolunu test etmek için Settings → Logs
> bölümüne bir **Trigger crash** düğmesi ekler. Sürüm (release) derlemelerinde
> devre dışı bırakılır.

## Optimization

Kopuz, büyük kütüphanelerde bile hızlı hissettirecek şekilde tasarlanmıştır.
Arka planda şunları yapıyoruz:

- **Zaten indekslenmiş olanları atla** - tarayıcı halihazırda gördüğü her yolun
  bir `HashSet`'ini tutar, böylece yeniden taramalar yalnızca yeni dosyaları
  işler. Eğer 10.000 parçanız varsa ve ardından 5 yeni parça eklerseniz, Kopuz
  diğer 9995 parçayı yeniden okumaz. Bu, özellikle HDD'lerde büyük bir fark
  yaratır.

- **Parallel startup loading** - başlatıldığında kütüphane, config, playlist'ler
  ve favorilerin tümü `tokio::join!` ile paralel olarak yüklenir. Bu
  değişiklikten önce her şey sırayla yükleniyordu ve bir süre boş bir pencereye
  bakıyordunuz. Şimdi neredeyse anında açılıyor.

- **Album art caching** - kapak resimleri bir kez çıkarılır ve diske kaydedilir
  (Linux'ta `~/.cache/kopuz/covers/`, macOS'ta `~/Library/Caches/`). Ayrıca
  ilerleme çubuğu her güncellendiğinde görüntüyü yeniden çözmemesi için macOS
  now-playing kapak resmi nesnesini bellekte önbelleğe alıyoruz.

- **Lazy loading images** - albüm kapakları arama sonuçlarında, parça
  satırlarında ve tür görünümlerinde `loading="lazy"` kullanır, böylece büyük
  bir kütüphanede gezinirken aynı anda yüzlerce görüntüyü yüklemeyiz.

- **Non-blocking I/O** - tüm ağır işler (metadata ayrıştırma, dosya tarama,
  kütüphane durumunu kaydetme) `spawn_blocking` iş parçacıklarında çalışır,
  böylece UI asla donmaz. Tam bir kütüphane taraması sırasında bile ana iş
  parçacığı duyarlı kalır.

- **Smarter sorting** - kütüphane görünümleri için normal `sort_by_key` yerine
  `sort_by_cached_key` kullanıyoruz, bu da her karşılaştırmada sıralama
  anahtarını (örneğin `.to_lowercase()`) yeniden hesaplamayı önler. Küçük bir
  şey gibi görünebilir ama binlerce parça olduğunda fark yaratır.

- **HTTP caching for artwork** - özel `artwork://` protokolü, resimleri
  `Cache-Control: public, max-age=31536000` ile sunar, böylece Webview zaten
  sahip olduğu kapakları yeniden talep etmez.

Genel olarak bu değişiklikler, rescan süresini önemli ölçüde azalttı ve
uygulama, özellikle 5000'den fazla parçaya sahip kütüphanelerde çok daha duyarlı
hissettiriyor. Bellek kullanımı da makul kalıyor, çünkü kodu çözülmüş
görüntüleri bellekte ihtiyaç duyulandan daha uzun süre tutmuyoruz.

## Tech Stack

- **Dioxus**: UI Framework
- **Symphonia**: Ses kod çözme kütüphanesi
- **Cpal**: Ses giriş/çıkış kütüphanesi
- **Lofty**: Metadata ayrıştırma
- **TailwindCSS**: CSS tabanlı stil framework'ü

## Crypto Donation

- **Solana**: "BK84dVEMnGBP5Tya2mEaB1BQgcSBjngf1NBmRCqefxGg"
- **Bitcoin**: "bc1qz94yz9xvufa6hxlvjzaajgd2zyfu86arn68hu4"
- **Monero**:
  "86mz3HxTrKyYpuvx78m6pufbXdwAnoyoZBztz6HyYrnM1XP5YVrMy9jTVRY5vzgGtkizACLpFwHEdafKTMoj6y8mAVgvWMz"
- **Ethereum**: "0xa490D50470cdFf837B6663F7f6cBe50B157224e5"
- **Solana Ağında USDT**: "GYmnAcrA5MbF6cUxT2m5d5cwdfr14qSY9WFYRwXxaibW"

## Credits

- Logo tasarımı: Lucas Amorim -
  [Instagram Hesabı](https://www.instagram.com/yattets/)

## Star History

[![Star History Chart](https://api.star-history.com/chart?repos=Kopuz-org/kopuz&type=date&legend=top-left)](https://www.star-history.com/?repos=Kopuz-org%2Fkopuz&type=date&legend=top-left)
