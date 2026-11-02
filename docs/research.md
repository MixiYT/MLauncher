# Legacy Launcher: репозитории и лицензии

Проверено 1 ноября 2026 года через GitHub API и файлы лицензий. GitHub-поиск выполнен по `legacylauncher in:name`, `tlauncher legacy in:name`, `legacy launcher license:mit minecraft`. Совпадение названия не доказывает связь с приложением на скриншоте.

| Репозиторий | Что это | Лицензия / решение |
| --- | --- | --- |
| [LegacyLauncher/archive](https://github.com/LegacyLauncher/archive) | Архив исходников нужного Legacy Launcher; описание упоминает резервные копии, декомпиляцию JAR и git-архивы | [LICENSE.txt](https://github.com/LegacyLauncher/archive/blob/master/LICENSE.txt): «more of an All rights reserved». **Не MIT**, код не используется |
| [LegacyLauncher/translations](https://github.com/LegacyLauncher/translations) | Переводы Legacy | GitHub API не указал лицензию; не использовать как свободно лицензированные ресурсы |
| [LegacyLauncher/docs](https://github.com/LegacyLauncher/docs) | Документация | GitHub API: CC-BY-SA-4.0; не лицензия на сам лаунчер |
| [gradenGnostic/LegacyLauncher](https://github.com/gradenGnostic/LegacyLauncher) | Другой лаунчер для Minecraft Legacy Console Edition, JavaScript, архивирован | [Проверенный текст MIT](https://github.com/gradenGnostic/LegacyLauncher/blob/main/LICENSE), Copyright (c) 2026 gardenGnostic. Не тот Legacy, код не используется |
| [MCPHackers/LaunchWrapper](https://github.com/MCPHackers/LaunchWrapper) | Java launch wrapper для старых версий Minecraft, не готовый аналог интерфейса | [Проверенный текст MIT](https://github.com/MCPHackers/LaunchWrapper/blob/main/LICENSE), Copyright (c) 2023 MCPHackers. Код не используется |
| [spoutcraft/LegacyLauncher](https://github.com/spoutcraft/LegacyLauncher) | Старый Spoutcraft-лаунчер, не Legacy с референса | GitHub API: NOASSERTION, не считать MIT |
| [betacraftuk/betacraft-launcher](https://github.com/betacraftuk/betacraft-launcher) | Лаунчер для старых Minecraft | GitHub API: GPL-3.0, не MIT |

Дальнейшие кандидаты, помеченные GitHub API как MIT: [davi-x86/LegacyConsoleLauncher](https://github.com/davi-x86/LegacyConsoleLauncher), [neoapps-dev/legacy-launcher](https://github.com/neoapps-dev/legacy-launcher). Это также проекты Legacy Console Edition, а не исходники указанного Legacy Launcher. Перед заимствованием необходимо отдельно проверять лицензии каждого файла и ресурсов.

## Вывод

MLauncher написан самостоятельно. Компактная панель и холодная палитра используются как общий визуальный ориентир. Код, логотип, фон и переводы Legacy Launcher не переносились. Обнаружение MIT-форка само по себе не снимает ограничения с исходного кода, если автор форка не имел права менять его лицензию.

## Официальные API

- [Манифест версий Mojang](https://piston-meta.mojang.com/mc/game/version_manifest_v2.json): полный официальный каталог `release`, `snapshot`, `old_beta`, `old_alpha`. MLauncher загружает его целиком, фильтрует на клиенте и хранит кеш. Категории не выводятся из номера версии эвристически.
- [Modrinth API](https://docs.modrinth.com/api/): публичный поиск и файлы доступны без токена; нужен идентифицирующий User-Agent, необходимо учитывать HTTP 429.
- [CurseForge API](https://docs.curseforge.com/rest-api/): `x-api-key`, Minecraft gameId 432, mods classId 6. Nullable downloadUrl не обходится.
- [Ely.by authentication](https://docs.ely.by/en/minecraft-auth.html): HTTPS `POST /auth/authenticate`, поддержка 2FA через `password:token`, отзыв через `/auth/invalidate`.
- [Ely.by authlib-injector](https://docs.ely.by/en/authlib-injector.html): Java-agent нужен отдельно от входа в аккаунт.
- [Ely.by skins](https://docs.ely.by/en/skins-system.html): текстуры по нику, профили, подписи. Для оффлайн-варианта одного аккаунтного флажка недостаточно.
- [Microsoft device-code flow](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-device-code): public client, код подтверждения в браузере, интервал опроса и срок действия кода.
- [Minecraft Microsoft authentication](https://minecraft.wiki/w/Microsoft_authentication): OAuth → Xbox Live → XSTS → Minecraft, требования к разрешению Client ID для Minecraft API, проверка Java entitlement и игрового профиля. Реализация самостоятельная, код примеров не копировался.
