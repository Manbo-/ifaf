# ifaf-susie

64bit版Susie互換ホスト向けの、Affinity `.af` プレビュー画像読み込みプラグインです。Affinity本体を起動せず、元ファイルを変更せずに画像を読み込みます。レイヤーや編集データの解析は行いません。

## ビルド

必要なもの: Rust stable の `x86_64-pc-windows-msvc` ターゲット、および Visual Studio Build Tools / MSVC リンカー。依存するRustクレートのバージョンは `Cargo.lock` に固定しています。

```powershell
cargo test --target x86_64-pc-windows-msvc --offline
.\build.cmd
```

プラグインは `dist\ifaf.sph` に出力されます。`.sph` は64bit x64版SusieプラグインAPIを実装するホストで使われる拡張子です。ホストの設定に従って、プラグイン用フォルダーに配置してください。

ビルドスクリプトを使わずにビルドする場合:

```powershell
cargo build --release --target x86_64-pc-windows-msvc --offline
```

DLL本体は `target\x86_64-pc-windows-msvc\release\ifaf_susie.dll` に生成されます。

## 対応形式と動作

- 対応するのは `.af` のみです。`.afphoto`、`.afdesign`、`.afpub` は `IsSupported` で受け付けません。
- ファイル先頭に、確認されているAffinity識別子 `00 FF 4B 41` が必要です。
- ファイル内のPNGシグネチャを順に調べ、`IEND` チャンクまでデコードできた最初のPNGを返します。候補が壊れている場合は読み飛ばし、次の候補を調べます。
- PNGをSusie向けの24bit・ボトムアップDIBに変換します。24bit DIBはSusieホスト間で広く扱えるため、透過ピクセルは白で合成します。
- ANSI版とUnicode版（関数名末尾が `W`）のSusie APIをエクスポートします。`W` 関数はUnicodeパスを扱います。
- `GetPreview` は実装していません。必須APIの `GetPicture` を使用してください。

Affinityのファイル形式は独自形式で、公開仕様はありません。このプラグインはファイル識別子と、内部に埋め込まれた妥当なPNGだけを利用します。圧縮された文書構造を推測・解析しません。提供された `sample.af` は識別子で始まり、841×1024ピクセルのPNGを含みます。このファイルにはPNG候補が1つだけのため、複数候補時の選択動作は生成したテストデータで検証しています。

## リソース上限とエラー時の動作

- 入力ファイルの上限: 512 MiB
- PNGの幅・高さの上限: それぞれ32,768ピクセル
- PNGデコーダーと返却ビットマップのメモリ上限: それぞれ128 MiB
- 1ファイル内で調べるPNGシグネチャの上限: 1,024個
- ファイルが途中で切れている、壊れている、上限を超えている、またはプレビューがない場合は、画像を返さずSusieのエラーコードを返します。FFIの各エントリーポイントでRustのパニックを捕捉します。
- ファイルは読み取り専用で開きます。

## 既存実装の調査と再利用方針

- [Asset-Hoard/afphoto](https://github.com/Asset-Hoard/afphoto) は、`00 FF 4B 41` の識別子と、ファイル内を走査してPNGを探す方法を説明しています。同プロジェクトはピクセル面積が最大のPNGを選びますが、READMEに記載されたテスト対象は `.afphoto` であり、その選択方法が `.af` にも適切だとは確認できません。ライセンスはMITです。このプラグインはコードをコピーしていません。
- [Affinity Thumbnail Extractor](https://github.com/omega-slender/affinity-thumbnail-extractor) は埋め込みPNGを走査し、既定ではバイト数が最小のPNGを選びます。`.af` を入力形式に挙げていますが、選択方法はAsset-Hoardと異なります。ライセンスはCC BY-NC 4.0のため、コードは再利用していません。
- [nextcloud-preview-affinity](https://github.com/udoschmitz/nextcloud-preview-affinity) は最後に見つかったPNGシグネチャと `IEND` を使って切り出し、プレビューがない場合は正常な失敗として扱います。Affinity 3の`.af`対応を記載しています。ライセンスはAGPL-3.0-or-laterのため、コードは再利用していません。
- [Affinity File Preview and Image Extractor](https://johnmcm.com/open-file/affinity) は文書全体のプレビューと配置画像を別々に扱います。同ページでは`.af`の処理は未検証とされているため、`.af`のプレビュー選択方法の根拠にはしていません。再利用可能なソースコードのライセンスは確認できませんでした。
- Susie APIのABIは[TOROの64bit SusieプラグインAPI資料](https://toroidj.github.io/dlsphapi.html)を参考にしています。`00IN`、`__stdcall`相当のWindowsシステムABI、x64版`PictureInfo`の配置、`LocalAlloc`で確保する返却ハンドル、ボトムアップ24bit DIBに従っています。

複数のPNGがある場合は、依頼に基づきファイル内で最初に見つかった有効なPNGを選びます。既存実装にあるサイズ順や最後の候補を選ぶ規則は採用していません。

## 依存関係とライセンス

PNGデコーダーには [`png` 0.18.1](https://crates.io/crates/png) を使用しています。ライセンスはMITまたはApache-2.0です。推移的依存関係と各バージョンは `Cargo.lock` に記録しています。プラグインのソースコードはMITライセンスです。詳細は `LICENSE` を参照してください。

## 検証結果

- `cargo test --target x86_64-pc-windows-msvc --offline`: 7件すべて成功。識別子の不一致、プレビューなし、`.af` の拡張子判定、最初の候補の選択、不正PNG候補の読み飛ばし、寸法上限、提供されたサンプルのデコードを確認しました。
- `cargo build --release --target x86_64-pc-windows-msvc --offline`: 成功。
- DLLのエクスポート名とx64形式を確認しました。
- 一時的なWindows ABI検証プログラムからDLLを読み込み、`sample.af` を使って `GetPluginInfo`、`IsSupportedW`、`GetPictureInfoW`、`GetPictureW` を呼び出しました。メタデータとDIBは841×1024、24bitで返ることを確認しました。
- Susie互換ホスト上での実表示も確認済みです。
- 長いパス、プレビューのない実ファイル、破損した実ファイル、上限を超える実ファイルは、まだ一連の動作として検証していません。
