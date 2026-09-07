# Hyperbola

[![ci](https://github.com/UltimateBomb/hyperbola/actions/workflows/ci.yml/badge.svg)](https://github.com/UltimateBomb/hyperbola/actions/workflows/ci.yml)
[![License: GPL-3.0](https://img.shields.io/badge/License-GPL--3.0-blue.svg)](LICENSE)
[![Windows + Android](https://img.shields.io/badge/работает%20на-Windows%20%7C%20Android-success.svg)](#скачать)
[![Latest release](https://img.shields.io/github/v/release/UltimateBomb/hyperbola?label=версия)](https://github.com/UltimateBomb/hyperbola/releases/latest)

**Скачивает видео и звук с сотен сайтов — на компьютере и на телефоне, из одного движка.**

Вставил ссылку, выбрал видео или только звук, нажал скачать. Приложение само
держит свежими себя, yt-dlp и ffmpeg — поэтому не протухает молча, как это
бывает со скачивальщиками, когда сайт меняет защиту.

🇬🇧 [In English](README.md)

---

## Скачать

| Что у вас | Берите это | Размер |
|---|---|---|
| **Windows** 10 или 11 | [**Скачать установщик**](https://github.com/UltimateBomb/hyperbola/releases/latest/download/Hyperbola_0.1.1_x64-setup.exe) | 3 МБ |
| **Телефон на Андроиде** (почти любой с 2015 года) | [**Скачать APK**](https://github.com/UltimateBomb/hyperbola/releases/latest/download/app-arm64-release.apk) | 60 МБ |
| Старый или 32-битный Андроид | [`app-arm-release.apk`](https://github.com/UltimateBomb/hyperbola/releases/latest/download/app-arm-release.apk) | 53 МБ |
| Эмулятор или планшет на x86 | [`app-x86_64-release.apk`](https://github.com/UltimateBomb/hyperbola/releases/latest/download/app-x86_64-release.apk) | 63 МБ |

**[→ Все файлы на странице релизов](https://github.com/UltimateBomb/hyperbola/releases/latest)**

*Windows:* запустите установщик. Всё остальное — yt-dlp и ffmpeg —
приложение скачает само при первом запуске.

*Андроид:* откройте APK и разрешите установку. Весь движок лежит внутри
пакета: настраивать нечего, Google Play не нужен. Если сомневаетесь, какой
файл брать — берите `app-arm64-release.apk`, это почти все телефоны.

## Как выглядит

<p align="center">
  <img src="docs/images/phone-analyze.png" width="30%" alt="Разбор ссылки" />
  <img src="docs/images/phone-queue.png" width="30%" alt="Очередь" />
  <img src="docs/images/phone-updates.png" width="30%" alt="Центр обновлений" />
</p>

## Что умеет

- **Видео со звуком или только звук** — выбор стоит до вставки ссылки, а не после.
- **Форматы, которые действительно играют.** YouTube отдаёт «лучшим» AV1 и
  Opus, а телефон старше нескольких лет не умеет их декодировать: файл
  скачивается прекрасно и не открывается. Здесь первыми идут H.264 и AAC, а
  для желающих новых кодеков есть переключатель.
- **Один центр обновлений** для приложения, yt-dlp и ffmpeg — с одной кнопкой.
  Провалившаяся проверка так и пишется: она никогда не выглядит как «всё свежо».
- **Очередь переживает закрытие.** Закройте приложение на середине — вернётся
  на паузе, недокачанный файл цел, продолжит с того же места.
- **Плейлисты** читаются примерно за секунду, галочками отмечаете нужное.
- **На телефоне:** загрузка продолжается с погасшим экраном, готовые файлы
  ложатся в выбранную вами папку, а одна кнопка отдаёт файл в Bluetooth или
  мессенджер — либо раздаёт по Wi-Fi, чем фильм попадает на автомагнитолу за
  полминуты вместо сорока.

## Как устроено

**Ядро не знает платформы. Оболочки не знают правил.**

`hyperbola-core` собирает команды yt-dlp, разбирает его вывод, ведёт очередь и
решает, что устарело. Оно не делает ввода-вывода вообще: ни процессов, ни
сокетов, ни файлов. Всё платформенное живёт в оболочке:

- **Windows** запускает `yt-dlp.exe` и сам держит его и ffmpeg свежими.
- **Андроид** не может запустить скачанный бинарник в принципе, поэтому движок
  — yt-dlp, Python, ffmpeg, QuickJS — лежит внутри APK и обновляет свой
  экстрактор из того же источника, что и десктоп.

Поэтому это одно приложение, а не два похожих: правило, закреплённое в ядре,
действует на обеих платформах, в одном релизе, с теми же тестами за спиной.

Подробнее — [ARCHITECTURE.md](ARCHITECTURE.md), про Андроид —
[docs/ANDROID.md](docs/ANDROID.md), про ключ подписи —
[docs/SIGNING.md](docs/SIGNING.md).

## Собрать самому

```bash
cargo test                       # ядро и запускатель
npm ci && npm --prefix ui ci
npm run build                    # приложение для компьютера
```

Без окна, когда до машины есть только консоль:

```bash
cargo run -p hyperbola-cli -- probe <ссылка>
cargo run -p hyperbola-cli -- get <ссылка> --max-height 1080 --out ~/Downloads
```

Для Андроида нужны JDK, Android SDK и NDK:

```bash
npx tauri android init && node scripts/android-prepare.mjs
npx tauri android build --apk --split-per-abi
```

## Откуда это выросло

Два проекта задали форму, но ни один не форкнут — кода из них не взято:

- **[Parabolic](https://github.com/NickvisionApps/Parabolic)** (MIT) — идея
  платформонезависимого ядра с тонкими оболочками и глубина работы с yt-dlp.
- **[Open Video Downloader](https://github.com/StefanLobbenmeier/youtube-dl-gui)**
  (AGPL-3.0) — привычка держать yt-dlp **и** ffmpeg свежими в работе, а не
  замораживать их при установке.

Само скачивание — [yt-dlp](https://github.com/yt-dlp/yt-dlp), склейка и
извлечение звука — [ffmpeg](https://ffmpeg.org/). На Андроиде оба приходят
через [youtubedl-android](https://github.com/yausername/youtubedl-android).

## Лицензия

GPL-3.0-or-later — приложение поставляется вместе с ffmpeg.

Видео на YouTube и других сайтах может быть защищено авторским правом.
Инструмент не одобряет и его авторы не отвечают за использование, нарушающее
эти законы.
