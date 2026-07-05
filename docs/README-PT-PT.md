<!--markdownlint-disable MD013 MD033 MD041 -->
<div align="center">
  <img src="../crates/kopuz/assets/banner.png" alt="Logótipo do Kopuz" height="300"/>
  <h1>Kopuz</h1>
  <p>
    Kopuz é uma aplicação moderna e leve de reprodução de música construída em Rust
    e com a framework Dioxus. Procura ainda fornecer uma interface limpa e responsiva
    para que possa gerir e desfrutar da sua coleção de música local.
  </p>
  <a href="https://discord.gg/K6Bmzw2E4M">
    <img src="https://img.shields.io/badge/Discord-5865F2?style=flat&logo=discord&logoColor=white" alt="Discord">
  </a>
  <img src="https://img.shields.io/badge/Rust-000000?style=flat&logo=rust&logoColor=white" alt="Rust">
  <img src="https://github.com/Kopuz-org/kopuz/actions/workflows/build.yml/badge.svg" alt="Build">
  <img src="https://github.com/user-attachments/assets/3234bb1b-6702-4802-946a-32f46e63d225" alt="Kopuz">

<br/>
  <br/>
  <p>
    <a href="../README.md">English</a> | <a href="../README-TR.md">Türkçe</a> | <b>Português de Portugal</b>
  </p>
</div>

## Sobre o Nome

O _kopuz_ é um antigo instrumento de cordas turco e é frequentemente considerado
o antepassado de muitos alaúdes da Ásia Central. Era tradicionalmente usado por
bardos e xamãs.

O _komuz_ quirguiz não é o mesmo instrumento, mas provavelmente um descendente
do _kopuz_. O _kobyz_ cazaque também está relacionado, embora seja tocado com
arco em vez de dedilhado. Em contraste, o _xomus_ tuvano/iacute (harpa de boca)
não está relacionado, apesar do nome semelhante.

Na lenda turca, o _kopuz_ está ligado a Dede Korkut, um bardo lendário, embora
isto seja mitológico e não histórico.

## Visão Geral

O Kopuz permite-lhe pesquisar os seus diretórios locais por ficheiros de áudio, transmitir a partir
do seu servidor Jellyfin ou Subsonic (Navidrome, etc.), ou ligar o **YouTube Music**
ou o **SoundCloud** como backend de streaming, de forma a organizar automaticamente tudo
numa biblioteca navegável. Pode pesquisar por artistas, álbuns, géneros, ou
explorar as suas próprias playlists. A aplicação foi construída para desempenho e
integração com o seu ambiente de trabalho, utilizando o poder do Rust.

A biblioteca, playlists, favoritos e definições são armazenados numa base de
dados **SQLite** local (`kopuz.db`); a interface lê-a em tempo real para que as
alterações apareçam imediatamente. Cada fonte de média transporta as suas
próprias credenciais e os seus próprios favoritos.

## Funcionalidades

[jellyfin-plugin-listenbrainz]: https://github.com/lyarenei/jellyfin-plugin-listenbrainz

- **Temas**: Inclui suporte a temas dinâmicos para personalizar a aparência
  visual. Também pode criar o seu próprio tema personalizado do zero com
  controlo total das variáveis de cor.
- **Integração Nativa**: Integra-se com os controlos de média do sistema no
  Linux (MPRIS), macOS (Now Playing / Remote Command Center) e Windows (System
  Media Transport Controls).
- **Mini-Reprodutor**: Uma sobreposição compacta do reprodutor que pode alternar a partir da barra inferior
  para uma vista mais pequena do que está a tocar no momento.
- **Minimizar para a Bandeja**: Opcionalmente fechar para um ícone da bandeja do sistema em vez de
  fechar, para que a reprodução continue em segundo plano. Altere nas **Definições**.
  Requer a biblioteca appindicator no Linux (ver notas de Instalação).
- **Discord RPC**: RPC incorporado incluído!!!
- **Múltiplos Backends**: Transmita a partir do seu servidor Jellyfin ou
  compatível com Subsonic (Navidrome funciona muito bem), ligue o YouTube Music
  ou SoundCloud, ou simplesmente aponte-o para uma pasta local. Misture e
  combine como quiser. Cada fonte é exposta através de uma camada unificada
  `MediaSource`, e a interface adapta-se às capacidades de cada fonte (pesquisa,
  transferências, rádio, descoberta, sincronização de favoritos, etc.) em vez de
  codificar rigidamente o comportamento por serviço.
- **YouTube Music**: Backend de streaming completo com uma página **Descobrir** no estilo do Spotify (músicas recomendadas, playlists, álbuns, artistas e ambientes), perfis ricos de informação sobre os **artistas**
  (banner, melhores músicas, álbuns, singles, artistas relacionados),
  navegação por álbuns/playlists, e **rádio de mistura** ("iniciar rádio" a partir de qualquer faixa).
  Inicie sessão com a sua conta para a sua biblioteca, Músicas Gostadas e playlists - ou
  execute-o **anonimamente** (sem iniciar sessão) para navegar, pesquisar e reproduzir faixas públicas. Veja [Configuração do YouTube Music](#configuração-do-youtube-music).
- **SoundCloud**: Backend de streaming com pesquisa, reprodução de faixas (MP3 progressivo
  e Go+ AAC/HLS), as suas **faixas gostadas** como favoritos, playlists só de leitura, e
  gostar/não gostar. Adicionado através de uma sessão única no navegador num perfil completamente isolado. Veja
  [Configuração do SoundCloud](#configuração-do-soundcloud).
- **Suporte a Letras**: Desfrute de letras sincronizadas em tempo real e letras
  simples, completas com deslocamento automático para acompanhar a sua música.
- **Favoritos**: Marque faixas localmente ou sincronize favoritos com o seu
  servidor Jellyfin/Subsonic.
- **Playlists**: Crie e faça a gestão das suas próprias playlists, adicione faixas individuais ou
  álbuns inteiros de uma vez, e sincronize playlists com o seu servidor.
- **Navegação por Género**: Navegue pela sua biblioteca por género tanto para música local como do servidor.
- **Emblemas de Tipo de Ficheiro**: As faixas locais mostram um pequeno emblema de formato (MP3, FLAC, WAV,
  etc.) nas linhas de faixas para que possa ver o formato de origem de maneira rápida.
- **Pesquisa**: Pesquise por artistas, álbuns e faixas com resultados em tempo real.
- **Registos de Audição**: As faixas contam as reproduções localmente para que possa ver o que
  realmente ouve mais.
- **Scrobbling**: Envie scrobbles para o ListenBrainz. Para utilizadores de Jellyfin,
  [jellyfin-plugin-listenbrainz] é recomendado usar vários clientes.
- **Suporte de Idiomas**: Interface disponível em Inglês, Russo, Alemão, Francês,
  Espanhol, Turco, Ucraniano, Polaco, Árabe, Grego, Hebraico, Húngaro,
  Indonésio, Japonês, Coreano, Romeno, Português do Brasil, Português de Portugal, Toki Pona e
  Chinês Simplificado com uma experiência simplificada para adicionar novos idiomas.
- **Alto Desempenho**: Processamento pesado em segundo plano e um scanner de biblioteca otimizado garantem que a aplicação abre instantaneamente, funciona sem problemas e salta rapidamente ficheiros previamente indexados.
- **Limpeza Automática**: Remove automaticamente faixas em falta ou eliminadas da sua
  biblioteca ao pesquisar novamente.
- **Navegação Suave**: Desfrute de uma interface polida onde as posições de scroll reiniciam
  corretamente ao navegar por diferentes vistas e páginas.
- **Reduzir Animações**: Definição de acessibilidade para reduzir efeitos de movimento se
  preferir uma interface mais minimalista.
- **Equalizador**: Equalizador incorporado de 5 bandas com predefinições e definições personalizadas para
  afinar o seu som.
- **Crossfade**: Mistura transições de faixas para uma reprodução automática mais suave entre
  músicas em compilações nativas de ambiente de trabalho. A reprodução no navegador atualmente usa uma troca normal de faixas.
- **Modo de Canal**: Alterne entre modos de saída `Estéreo`, `Mono`, `Apenas Esquerdo`, `Apenas Direito`,
  e `Trocar E/D`.
- **Integração com yt-dlp**: Transfira áudio diretamente do YouTube e outros
  sites suportados via yt-dlp. Escolha o seu formato de saída (Melhor Áudio, MP3, FLAC,
  WAV ou vídeo MP4). O FLAC não é recomendado pois o yt-dlp gera áudio com perdas
  em vez de descodificar de uma fonte sem perdas. Suporta SponsorBlock, divisão de capítulos, cookies, limitação de taxa e mais. Requer `yt-dlp` instalado no seu sistema.
- **Definições de Metadados**: Uma secção dedicada de Metadados nas Definições permite-lhe
  controlar como as imagens de artistas são obtidas. Escolha entre **Capa de Álbum** (usa
  a primeira capa de álbum como foto do artista, predefinido) ou **Foto de Artista**
  (obtém imagens reais de artistas diretamente do seu servidor Jellyfin ou Subsonic).
  Ao mudar para o modo Foto de Artista, as imagens são obtidas do servidor em
  segundo plano assim que abrir a página de Artistas. Se um artista não tiver uma foto dedicada
  no seu servidor, a primeira capa de álbum é usada como alternativa para que nada
  apareça em branco.

## Instalação

### Cargo (crates.io)

Instale diretamente com o Cargo:

```bash
cargo install kopuz
```

### NixOS / Nix

**Executar diretamente sem instalar:**

```bash
nix run github:temidaradev/kopuz
```

**Instalar no seu perfil:**

```bash
nix profile add github:temidaradev/kopuz
```

**No NixOS, com o flake:**

> [!TIP]
> Isto é recomendado em vez de `nix profile` pois instala o Kopuz como uma
> aplicação de sistema adequada com ícone e entrada `.desktop`.

Adicione o Kopuz aos inputs do seu `flake.nix`:

```nix
{
  inputs.kopuz.url = "github:temidaradev/kopuz";
}
```

Depois passe-o para a configuração do seu sistema e adicione o substituto Cachix
para que descarregue o binário pré-compilado em vez de compilar:

```nix
{
  nix.settings = {
    substituters      = ["https://kopuz.cachix.org" ];
    trusted-public-keys = ["kopuz.cachix.org-1:J2X3AnAYhKTJW5S3aCLoA1ckonQXVNZMQvhZA0YAufw="];
  };
}
```

Depois instale o pacote:

```nix
{pkgs, kopuz, ...}: let
  kopuzPkg = kopuz.packages.${pkgs.stdenv.hostPlatform.system}.default

in {
  environment.systemPackages = [kopuzPkg];
}
```

### AUR (Arch Linux)

Instale a partir do AUR usando o seu ajudante preferido:

```bash
yay -S kopuz-bin
# or
paru -S kopuz-bin
```

### Flatpak (Recomendado)

O Kopuz estará em breve disponível no Flathub. Para instalar a partir do
manifesto fonte:

```bash
git clone https://github.com/temidaradev/kopuz
cd kopuz
flatpak-builder --user --install --force-clean build-dir packaging/flatpak/com.temidaradev.kopuz.json
flatpak run com.temidaradev.kopuz
```

Também pode clicar no ficheiro e abri-lo com um fornecedor de aplicações, por
exemplo o KDE discover

### AppImage

> [!IMPORTANT]
> O AppImage requer ambos o `webkit2gtk-4.1` e o `gtk3` instalados no seu sistema.
> Essas dependências não estão incluídas. O ícone da bandeja do sistema adicionalmente
> precisa da biblioteca **appindicator** (ex. `libayatana-appindicator`); sem
> ela o Kopuz funciona bem mas não mostra o ícone da bandeja.
>
> Na maioria das distribuições com um ambiente de trabalho moderno, estas já
> estão presentes. Terá de as instalar manualmente se ainda não estiverem
> instaladas.

Em distribuições baseadas em Arch, se o AppImage falhar com um erro
`WebKitNetworkProcess`, execute-o com:

```bash
LD_LIBRARY_PATH=/usr/lib ./kopuz_*.AppImage
```

Ou crie symlinks uma vez (requer sudo):

```bash
sudo mkdir -p /usr/libexec/webkit2gtk-4.1
sudo ln -s /usr/lib/webkit2gtk-4.1/WebKitNetworkProcess /usr/libexec/webkit2gtk-4.1/
sudo ln -s /usr/lib/webkit2gtk-4.1/WebKitWebProcess /usr/libexec/webkit2gtk-4.1/
sudo ln -s /usr/lib/webkit2gtk-4.1/WebKitGPUProcess /usr/libexec/webkit2gtk-4.1/
```

### Compilar a partir do Código-Fonte

#### Dependências

**Ao Usar Nix**

> [!TIP]
> [Nix](https://nixos.org) é o principal meio de desenvolvimento para o Kopuz, e
> é o método recomendado para obter dependências de compilação num ambiente
> puro, reprodutível e consistente entre sistemas.

```bash
# Using Nix3 CLI
nix develop
```

Se for utilizador de [Direnv](https://direnv.net) use o `.envrc` fornecido:

```bash
# Using Direnv
direnv allow
```

O Direnv é recomendado se quiser continuar a usar a sua shell de utilizador
dentro do ambiente de desenvolvimento.

> [!NOTE]
> O ícone da bandeja do sistema (usado por **minimizar para a bandeja**) requer
> a biblioteca **appindicator** em tempo de execução. Está incluída nas
> dependências do pacote abaixo. Sem ela o ícone da bandeja simplesmente não
> aparece e fechar a janela sai da aplicação em vez de a ocultar - o Kopuz
> continua a funcionar normalmente. A shell de desenvolvimento Nix já a fornece.

**Sistemas Baseados em Arch Linux**

```bash
sudo pacman -S rust cargo dioxus-cli base-devel cmake pkgconf opus alsa-lib xdotool webkit2gtk-4.1 gtk3 libsoup3 openssl libayatana-appindicator
```

**Sistemas Baseados em Debian**

```bash
sudo apt install rustc cargo build-essential cmake pkg-config libopus-dev libasound2-dev libxdo-dev libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libssl-dev libayatana-appindicator3-1
cargo install dioxus-cli
```

**Sistemas Baseados em Fedora**

```bash
sudo dnf groupinstall "Development Tools" "Development Libraries"
sudo dnf install rust cargo cmake pkgconf-pkg-config opus-devel alsa-lib-devel libxdo-devel webkit2gtk4.1-devel gtk3-devel libsoup3-devel openssl-devel libayatana-appindicator-gtk3
cargo install --locked dioxus-cli
```

**Sistemas Baseados em openSUSE**

```bash
sudo zypper install rust cargo cmake pkg-config libopus-devel alsa-devel xdotool webkit2gtk3-soup2-devel gtk3-devel libsoup3-devel libopenssl-devel libayatana-appindicator3-1
cargo install --locked dioxus-cli
```

#### Desenvolver o Kopuz

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

**Nota sobre quarentena:** Se descarregou um `.dmg` em vez disso, o macOS pode
bloqueá-lo. Execute uma vez para limpar a flag de quarentena:

```bash
xattr -d com.apple.quarantine /Applications/Kopuz.app
```

### Onde o Kopuz guarda os seus ficheiros?

As suas definições, biblioteca pesquisada, playlists e favoritos vivem todos
numa única base de dados **SQLite**, `kopuz.db`, no diretório de configuração.
As capas de álbuns e faixas transferidas ficam em disco no diretório de cache.
(As compilações de debug usam uma `kopuz-debug.db` separada para que `dx serve`
nunca toque nos seus dados reais. Pode substituir a localização da BD com a
variável de ambiente `KOPUZ_DB_PATH`.)

No **macOS**:

- `~/Library/Application Support/com.temidaradev.kopuz/kopuz.db` - definições,
  biblioteca, playlists, favoritos
- `~/Library/Caches/com.temidaradev.kopuz/covers/` - capas de álbuns em cache
- `~/Library/Caches/com.temidaradev.kopuz/offline_tracks/` - faixas transferidas

No **Linux** (especificação XDG):

- `~/.config/kopuz/kopuz.db` - definições, biblioteca, playlists, favoritos
- `~/.cache/kopuz/covers/` - capas de álbuns em cache
- `~/.cache/kopuz/offline_tracks/` - faixas transferidas

No **Windows** (AppData):

- `%APPDATA%\temidaradev\kopuz\config\kopuz.db` - definições, biblioteca,
  playlists, favoritos
- `%LOCALAPPDATA%\temidaradev\kopuz\cache\covers\` - capas de álbuns em cache
- `%LOCALAPPDATA%\temidaradev\kopuz\cache\offline_tracks\` - faixas transferidas

> [!NOTE]
> A atualizar a partir de uma versão mais antiga? No primeiro arranque o Kopuz importa o seu já existente
> `library.json` e `playlists.json` para o `kopuz.db`, deixando cópias de segurança `*.json.bak`
> para trás. Os ficheiros JSON antigos deixam de ser lidos depois disso.

Se as capas não estiverem a aparecer ou a biblioteca parecer estranha, basta
apagar a pasta de cache e clicar em pesquisar novamente.

## Configuração do YouTube Music

O Kopuz pode usar o YouTube Music como backend de streaming. Adicione-o a partir
de **Definições → Servidores de média → Adicionar → YouTube Music**.

### Escolher um modo

O diálogo de configuração oferece dois métodos:

- **Iniciar sessão com um navegador** - o kopuz abre a página de início de
  sessão da Google num **perfil de navegador isolado** (uma sessão fresca e
  separada; a sua navegação normal nunca é tocada), espera que inicie sessão e
  extrai os cookies da sessão. Escolha qual navegador da família Chromium
  instalado usar (Chrome, Chromium, Brave, Edge ou Vivaldi). Isto desbloqueia a
  sua **biblioteca, Músicas Gostadas, playlists e artistas seguidos**.

- **Continuar sem iniciar sessão (anónimo)** - sem início de sessão, sem
  cookies. Pode **navegar, pesquisar, abrir páginas de artista/álbum/playlist,
  iniciar rádio de mistura e reproduzir faixas públicas**. Músicas Gostadas,
  playlists da biblioteca e seguir/gostar estão desativados (essas vistas
  mostram um prompt "inicie sessão para ativar"). Faixas exclusivas do Music
  Premium não podem ser reproduzidas anonimamente.

### Faixas Premium

As faixas bloqueadas pelo Music Premium recorrem a uma resolução local
[`yt-dlp`](https://github.com/yt-dlp/yt-dlp) quando o caminho principal
retorna `UNPLAYABLE`, por isso ter o `yt-dlp` instalado pode ajudar nessas situações. O modo anónimo não pode reproduzir conteúdo exclusivo do Premium.

## Configuração do SoundCloud

O Kopuz consegue ainda usar o SoundCloud como backend de streaming. Adicione-o a partir de **Definições → Servidores de
média → Adicionar → SoundCloud**.

Não existe URL ou palavra-passe para digitar. O Kopuz abre o endereço `soundcloud.com/signin` num
**perfil de navegador isolado** (uma sessão limpa e completamente separada; a navegação normal
nunca é tocada), apenas agurda que inicie sessão e obtém o `oauth_token` da sessão.
Escolha qual navegador da família Chromium instalado usar (Chrome, Chromium, Brave,
Edge ou Vivaldi).

Depois de iniciar sessão tem acesso à pesquisa, reprodução de faixas (MP3 progressivo mais streams Go+ AAC/HLS),
às suas **faixas gostadas** como favoritos, acesso só de leitura às suas
playlists, e a funcionalidade de gostar ou não gostar das faixas disponiveis. Remover a fonte limpa o seu perfil isolado.

## Registos e Debugging

O Kopuz regista através de [`tracing`](https://docs.rs/tracing). A maioria disto
é acessível a partir da própria aplicação - **Definições → Registos tem Abrir
pasta de registos, Exportar registos**, e um interruptor **Ativar Rastreio de
Desempenho** - por isso os utilizadores nunca precisam de um terminal para
enviar um relatório útil.

### Onde os ficheiros estão

Todos os ficheiros estão no diretório de registos (o botão **Abrir pasta de
registos** salta diretamente para aqui):

- Linux: `~/.cache/kopuz/logs/`
- macOS: `~/Library/Caches/com.temidaradev.kopuz/logs/`
- Windows: `%LOCALAPPDATA%\temidaradev\kopuz\cache\logs\`

| Ficheiro                | O que é                                                                                                         |
| ----------------------- | --------------------------------------------------------------------------------------------------------------- |
| `latest.log`            | A sessão atual. Temporização de spans + eventos; o registo em tempo real.                                       |
| `kopuz-<timestamp>.log` | Sessões anteriores, arquivadas no arranque (mantém as últimas 10). Um reinício nunca apaga a execução anterior. |
| `crash-<timestamp>.txt` | Escrito **apenas num crash** (pânico do Rust): mensagem, backtrace, cauda do registo recente, versão da app/SO. |
| `kopuz-trace.json`      | Rastreio de desempenho - apenas quando o rastreio está ativado (ver abaixo). Sobrescrito em cada execução.      |

Os timestamps são em UTC `YYYY-MM-DD_HH-MM-S`, por isso os ficheiros ordenam-se
cronologicamente.

### Guia rápido de triagem

**A aplicação crashou →** um `crash-<timestamp>.txt` é gerado automaticamente.
Peça ao utilizador **Definições → Registos → Exportar registos** (agrupa
`latest.log` + o relatório de crash mais recente num ficheiro), ou **Abrir pasta
de registos** e apanhe o `crash-*.txt` mais recente.

**Problema de desempenho (congestionamento / carregamento lento / gagueira) →**
peça ao utilizador para:

1. **Definições → Registos → ativar "Rastreio de Desempenho"**, depois
   **reiniciar** a aplicação (o interruptor avisa sobre isto - o gravador de
   rastreio é configurado uma vez no arranque).
2. Reproduzir a ação lenta.
3. **Sair da aplicação** (isto liberta o rastreio corretamente).
4. **Definições → Registos → Abrir pasta de registos** e enviar
   `kopuz-trace.json` (ou **Exportar registos**).

Abra o rastreio em [speedscope.app](https://speedscope.app) ou
[ui.perfetto.dev](https://ui.perfetto.dev). Os caminhos críticos (resolução de
stream do YouTube, navegação/pesquisa/paginação, rádio de mistura, pesquisa da
biblioteca, transferências, transições de reprodução, renders por componente)
estão instrumentados como spans nomeados, e o trabalho em threads de trabalhador
aninha-se sob a ação que o lançou, por isso o rastreio mostra exatamente onde o
tempo vai. Desative-o depois - adiciona overhead e aumenta o ficheiro de
rastreio durante sessões longas.

### Variáveis de ambiente para utilizadores avançados

A **verbosidade** dos registos é controlada por variáveis de ambiente para
execuções no terminal:

```bash
# Registos verbosos (nível debug) para uma sessão
KOPUZ_DEBUG=1 kopuz

# Detalhados, por módulo (sobrepõe KOPUZ_DEBUG); sintaxe de diretiva de tracing padrão
KOPUZ_LOG="server::ytmusic=trace,kopuz=debug" kopuz

# Perfilagem profunda da árvore de render: os próprios spans de render/diff por componente do Dioxus
# (ative primeiro o interruptor de rastreio nas Definições; isto só controla o que é registado)
KOPUZ_LOG="info,dioxus_core=trace" kopuz
```

`RUST_LOG` também funciona; `KOPUZ_LOG` tem precedência.

O **rastreio de desempenho** só é ativado via **Definições → Registos → Ativar
Rastreio de Desempenho** (depois reiniciar) - não há variável de ambiente para
isso; a interface é a única fonte de verdade. Desativado por predefinição → zero
overhead.

> As compilações de debug adicionam um botão **Desencadear crash** em Definições
> → Registos para exercitar o caminho do relatório de crash. Está compilado fora
> das compilações de release.

## Otimização

O Kopuz foi construído para se sentir rápido mesmo com bibliotecas grandes. Eis
o que fazemos por baixo do capô:

- **Saltar o que já está indexado** - o scanner mantém um `HashSet` de todos os
  caminhos que já viu, por isso as pesquisas só processam ficheiros novos. Se
  tiver 10.000 faixas, e depois adicionar 5 novas, o Kopuz não voltará a ler as
  outras 9995. Isto faz uma enorme diferença, especialmente em HDDs.

- **Carregamento paralelo no arranque** - no arranque, biblioteca, configuração,
  playlists e favoritos carregam todos em paralelo com `tokio::join!`. Antes
  desta mudança, tudo carregava sequencialmente e ficaria a olhar para uma
  janela em branco por um bocado. Agora é quase instantâneo.

- **Cache de capas de álbuns** - as imagens de capas são extraídas uma vez e
  guardadas em disco (`~/.cache/kopuz/covers/` no Linux, `~/Library/Caches/` no
  macOS). Também colocamos em cache o objeto de artwork now-playing do macOS na
  memória para que não volte a descodificar a imagem sempre que a barra de
  progresso atualiza.

- **Carregamento preguiçoso de imagens** - as capas de álbuns nos resultados de
  pesquisa, linhas de faixas e vistas de género usam todas `loading="lazy"` para
  que não estejamos a carregar centenas de imagens de uma vez quando percorre
  uma biblioteca grande.

- **I/O não bloqueante** - todo o trabalho pesado (análise de metadados,
  pesquisa de ficheiros, guardar estado da biblioteca) corre em threads
  `spawn_blocking` para que a interface nunca congele. O thread principal
  mantém-se responsivo mesmo durante uma pesquisa completa da biblioteca.

- **Ordenação mais inteligente** - usamos `sort_by_cached_key` em vez de
  `sort_by_key` normal para as vistas da biblioteca, o que evita recalcular a
  chave de ordenação (como `.to_lowercase()`) em cada comparação. Uma coisa
  pequena talvez, mas soma-se com milhares de faixas.

- **Cache HTTP para artwork** - o protocolo personalizado `artwork://` serve
  imagens com `Cache-Control: public, max-age=31536000` para que o Webview não
  volte a pedir as capas que já tem.

No geral, estas mudanças reduziram significativamente o tempo de pesquisa e a
aplicação sente-se muito mais responsiva, especialmente com bibliotecas com mais
de 5000 faixas. O uso de memória mantém-se razoável também já que não estamos a
guardar imagens descodificadas na memória mais tempo do que o necessário.

## Tecnológia

- **Dioxus**: Framework de Interface
- **Symphonia**: Biblioteca de descodificação de áudio
- **Cpal**: Biblioteca de I/O de áudio
- **Lofty**: Análise de metadados
- **SQLite / sqlx**: Armazenamento local com consultas verificadas em tempo de
  compilação
- **TailwindCSS**: Framework de estilização baseado em CSS

## Doações em Criptomoedas

- **Solana**: "2fapJYRztnTRLpJbmyEUnsuZ36AzLK2JrMmmLEfDqKpN"
- **Bitcoin**: "bc1qz94yz9xvufa6hxlvjzaajgd2zyfu86arn68hu4"
- **Monero**:
  "86mz3HxTrKyYpuvx78m6pufbXdwAnoyoZBztz6HyYrnM1XP5YVrMy9jTVRY5vzgGtkizACLpFwHEdafKTMoj6y8mAVgvWMz"
- **Ethereum**: "0xa490D50470cdFf837B6663F7f6cBe50B157224e5"
- **USDT na Cadeia Solana**: "GYmnAcrA5MbF6cUxT2m5d5cwdfr14qSY9WFYRwXxaibW"

## Créditos

- Design do logótipo por: Lucas Amorim -
  [His Instagram Account](https://www.instagram.com/yattets/)

## Histórico de Estrelas

[![Star History Chart](https://api.star-history.com/chart?repos=Kopuz-org/kopuz&type=date&legend=top-left)](https://www.star-history.com/?repos=Kopuz-org%2Fkopuz&type=date&legend=top-left)
