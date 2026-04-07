# Design System — Focus Vision PCVR

## Product Context
- **What this is:** オープンソースのPCVRストリーミングツール。VIVE Focus VisionでSteamVRゲームをワイヤレスプレイ
- **Who it's for:** Focus Visionユーザー、将来的に全スタンドアロンHMDユーザー
- **Space/industry:** VRストリーミング (ALVR, Virtual Desktop, Steam Link)
- **Project type:** システムツール (PCコンパニオンアプリ + HMDクライアント + ブランディング)

## Aesthetic Direction
- **Direction:** Brutally Minimal — ツールが消えてVR体験だけが残る
- **Decoration level:** minimal — タイポグラフィと余白だけで語る。装飾要素なし
- **Mood:** 透明で空気のような存在。テック感を抑え、静かに機能する道具
- **Reference sites:** vrdesktop.net, github.com/alvr-org/ALVR
- **Differentiation:** 競合は全員ダーク+ブルーのテック感。Focus Visionはエメラルドグリーン+セリフブランドで「体験を売る」

## Typography
- **Display/Hero:** Instrument Serif — ブランドマーク、ヒーローテキスト。VRツールでセリフは異例だが「テック製品」ではなく「体験」を伝える
- **Body/UI:** Geist — クリーンで現代的。読みやすく、UIに最適
- **Data/Tables:** Geist Mono (tabular-nums) — レイテンシー、FPS、ビットレート等の数値表示
- **Code:** Geist Mono
- **Loading:** Google Fonts (Instrument Serif) + CDN (Geist: cdn.jsdelivr.net/npm/geist)
- **Scale:**
  - 2xs: 11px — 極小ラベル
  - xs: 12px — セクションラベル、メタデータ
  - sm: 13px — 補助テキスト、ステータス
  - base: 15px — 本文
  - lg: 17px — リード文
  - xl: 24px — セクションヘッダー
  - 2xl: 36px — ページヘッダー
  - 3xl: 48px — ヒーロー/ブランド

## Color
- **Approach:** restrained — 1アクセント + ニュートラル。色は希少で意味がある

### Dark Mode (Primary)
| Token | Hex | Usage |
|-------|-----|-------|
| bg-primary | #0a0a0c | ページ背景 |
| bg-secondary | #111114 | カード、パネル |
| bg-tertiary | #1a1a1f | 入力フィールド背景、ホバー |
| bg-elevated | #222228 | モーダル、ドロップダウン |
| border | #2a2a32 | 標準ボーダー |
| border-subtle | #1e1e24 | 軽いセパレーター |
| text-primary | #e8e8ec | 見出し、本文 |
| text-secondary | #9898a4 | 補助テキスト |
| text-muted | #5a5a68 | プレースホルダー、ラベル |
| accent | #34D399 | アクション、ステータス「接続済み」 |
| accent-dim | #1a7a52 | アクセントのボーダー、微細な強調 |
| accent-bg | rgba(52,211,153,0.08) | アクセント背景のヒント |

### Light Mode
| Token | Hex | Usage |
|-------|-----|-------|
| bg-primary | #fafafa | ページ背景 |
| bg-secondary | #f0f0f2 | カード |
| bg-tertiary | #e4e4e8 | 入力フィールド |
| bg-elevated | #ffffff | モーダル |
| text-primary | #111114 | 見出し、本文 |
| text-secondary | #5a5a68 | 補助テキスト |
| accent | #059669 | 彩度下げたアクセント |

### Semantic Colors
| Token | Hex | Usage |
|-------|-----|-------|
| success | #34D399 | 接続済み、正常 |
| warning | #fbbf24 | ネットワーク品質低下 |
| error | #f87171 | 切断、エラー |
| info | #60a5fa | 情報通知 |

## Spacing
- **Base unit:** 8px
- **Density:** comfortable
- **Scale:**
  - 2xs: 2px
  - xs: 4px
  - sm: 8px
  - md: 16px
  - lg: 24px
  - xl: 32px
  - 2xl: 48px
  - 3xl: 64px

## Layout
- **Approach:** grid-disciplined — 縦一列、一画面一機能
- **Grid:** 単一カラム (PCアプリ: max 720px, HMDオーバーレイ: コンテンツ幅)
- **Max content width:** 720px (PCアプリ)
- **Border radius:**
  - sm: 4px — 入力フィールド、小ボタン
  - md: 8px — カード、パネル
  - lg: 12px — モーダル、モックアップ枠
  - full: 9999px — ステータスドット、トグル

## Motion
- **Approach:** minimal-functional — 状態変化と接続過程のみ
- **Easing:**
  - enter: cubic-bezier(0.16, 1, 0.3, 1) — ease-out
  - exit: ease-in
  - move: ease-in-out
- **Duration:**
  - micro: 50-100ms — ホバー、フォーカス
  - short: 150-250ms — ボタン状態変化
  - medium: 250-400ms — パネル展開、接続ステータス遷移
  - long: 400-700ms — 未使用（最小限方針）
- **唯一のアニメーション:** ステータスドットのpulse (接続中を示す)

## HMD Specific Guidelines
- 背景は半透明ブラー (rgba(10,10,12,0.85) + backdrop-filter: blur(20px))
- テキストサイズは通常の1.5倍以上 (VR内での可読性)
- 入力は大きなタッチターゲット (最小56x72px per digit)
- 情報は最小限。ステータスバーは1行: レイテンシー、FPS、ビットレートのみ

## Sleep Mode UI
- **暗転オーバーレイ:** フルスクリーン黒クワッド、alpha 0.0→0.85 を 400ms でフェードイン
- **復帰:** 動き検知で alpha 0.85→0.0 を 250ms でフェードアウト（即座の復帰感）
- **テキスト:** 暗転中は何も表示しない。ツールが消える哲学と一貫
- **コンパニオンアプリ:** ステータス行に "💤 Sleep" を text-muted で表示

## Face Tracking Status
- **コンパニオンアプリ:** OSC接続状態を小さなインジケーターで表示
  - 送信中: accent (#34D399) ドット + "FT Active"
  - 未接続: text-muted ドット + "FT Idle"
- **HMDオーバーレイ:** FTステータスは表示しない（最小限方針）

## Battery Indicator
- **HMDオーバーレイ:** 既存の信号バーの隣に数字で表示（例: "87%"）
  - 100-20%: text-secondary
  - 20-10%: warning (#fbbf24)
  - <10%: error (#f87171)
- **コンパニオンアプリ:** ステータスセクションにバッテリーアイコン + パーセント

## Controller Touch Feedback
- **コンパニオンアプリ:** 入力デバッグビューで touch 状態を accent-dim の丸で表示
- **デッドゾーン:** スティック表示時、デッドゾーン領域を bg-tertiary の円で視覚化

## Decisions Log
| Date | Decision | Rationale |
|------|----------|-----------|
| 2026-03-27 | Initial design system created | /design-consultation based on competitive research (ALVR, Virtual Desktop, SteamVR) |
| 2026-03-27 | Emerald green accent (#34D399) | 競合は全員ブルー。グリーンは「接続済み/正常」を自然に伝え、差別化になる |
| 2026-03-27 | Instrument Serif for brand | VRツールでセリフは異例。「テック製品」ではなく「体験」を売るメッセージ |
| 2026-03-27 | Brutally Minimal aesthetic | 「設定ゼロ」の製品哲学と一貫。ツールが消えてVR体験だけが残る |
| 2026-04-07 | Sleep mode: no text overlay | 暗転中はテキストなし。「ツールが消える」哲学と一貫 |
| 2026-04-07 | Battery: number-only display | アイコンではなく数字。VR内の可読性とミニマル方針 |
| 2026-04-07 | FT status: companion only | HMDオーバーレイには表示しない。情報最小限の原則 |
