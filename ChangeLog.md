# Change Log

See [How to write this file](docs/guides/release.zh-CN.md#changelog-syntax) for the syntax.

## v0.1.0

### english
- **Realms** (headline feature): each realm is a self-contained world with its own mods, saves and settings — several mod combos can live side by side without ever interfering, so switching combos no longer means uninstalling and reinstalling mods; a realm can also hide the maps bundled with the game
- **Mod view**: choose the mods you want on one screen — dependencies are ticked for you and conflicting mods turn red, so you can tell whether a combo will run before starting the game
- **Presets**: save a combo that works as a preset and switch between combos in one click; presets can also be exported and shared
- **Wiki**: look up factions, towns, creatures, heroes, artifacts, spells, skills and terrains — with images, without launching the game
- **Settings**: every VCMI option in one searchable panel — no more editing config files by hand
- **Also included**: interface in 5 languages (zh / en / ru / pl / de), one-click in-app update, and a portable install where all data lives beside the launcher

### chinese
- **领域系统**（核心能力）：一个领域就是一套独立的世界，模组、存档、设置各归各的——多套模组组合可以并存且互不干扰，换组合再也不用反复装卸模组；还能按领域单独屏蔽游戏自带地图
- **模组视图**：一屏挑选要用的模组——依赖自动帮你勾上、互相冲突的模组标红，开游戏前就能看出这套组合能不能跑
- **预设管理**：把一套能跑的模组存成预设，随时一键切换；也可以导出分享给别人
- **百科**：不开游戏也能查城镇、建筑、生物、英雄、神器、法术、技能等资料，带图
- **设置面板**：VCMI 的设置项全部搬进界面、支持搜索，不用再手改配置文件
- **其他能力**：5 种界面语言（中/英/俄/波/德）、一键检查更新升级、单文件夹便携部署（数据都在启动器旁边）

## v0.2.0

### english

**Improve the first-time startup wizard**:
 - One-click switching and importing of exclusive user data directories to avoid configuration conflicts with other game versions
 - It is more convenient to import original game files
 - More convenient for importing HD footage
 - It is more convenient to import battles from historical records

**Improve the encyclopedia**:
 - Supports the brand-new 'biological animation' effect, and supports new action combinations such as standby and free patrol
 - Supports a brand-new "wide screen" for a better viewing experience of creature animations, and supports a "neutral creature" screen, adapting to more third-party creature animations
 - Support for 'sorted categorical data', providing a more user-friendly data display
 - Support "category memory" to record the last viewed position
 - Support for the `Vocabulary` category and improvement of the display effect for the `All Modules` category
 - Improve support for 'HD materials', enabling high-definition display of more images
 - Generate cache faster, with more user-friendly real-time progress feedback for cache generation

**Fix**:
 - Fixed the issue where the hero's specialties were not displayed in the third-party module "Heroes"
 - Clear outdated configurations in `persistentStorage.json` before running the game to avoid causing it to crash

### chinese

**改进首次启动向导**：
  - 一键切换与导入专属用户数据目录，避免跟其他游戏版本出现配置冲突
  - 更方便导入原版游戏文件
  - 更方便导入HD高清素材
  - 更方便导入历代记战役

**改进百科**：
  - 支持全新`生物动画`效果，支持待机、自由巡逻等全新动作组合
  - 支持全新`宽幕布`，能更好查看生物动画，支持`中立生物`幕布，适配更多第三方生物动画
  - 支持`分类数据排序`, 更友好的数据展示
  - 支持`分类记忆`，记录上次看到的位置
  - 支持`词汇表`分类与改进`所有模组`分类显示效果
  - 改进`高清素材`支持，覆盖更多图片高清显示
  - 更快地生成缓存，更友好的缓存生成实时进度反馈

**修复**：
  - 修复第三方模组`英雄`没有显示英雄特长
  - 运行游戏前清理`persistentStorage.json`过时的配置避免导致游戏奔溃

## v0.2.1

### english

**Fixes:**
- Fixed the issue where the color loss of cached images in the module was caused by incorrect image cropping
- Fixed the logic for reading town names in multiple languages
- Fixed the issue where war machines (crossbow carts, first aid tents, supply carts) in the "Initial Troops" section of heroes were mistakenly placed in the initial troop grid
- Fixed the animation display of War Machine in the Encyclopedia
- Fixed the issue of some creature animations causing the screen to flicker
- Fixed the issue where the software download window would freeze due to high traffic
- Fix the issue where some background music for the faction parade cannot be played
- Compatible with the configuration methods of some town mods

### chinese

**修复：**
- 修复模组缓存图片被错误抠图导致图片丢失色彩
- 修复城镇名称多语言的读取逻辑
- 修复英雄「初始部队」里的战争机器（弩车 / 急救帐篷 / 补给车）被错分到初始部队格子里
- 修复百科里战争机器动画显示
- 修复一些生物动画幕布跳动
- 修复软件下载窗口繁忙卡死
- 修复阵营巡礼某些阵营背景音乐无法播放
- 兼容一些城镇模组的配置方式
