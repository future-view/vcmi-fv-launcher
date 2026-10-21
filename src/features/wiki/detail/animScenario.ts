//! 生物动画的**状态编排**（2026-10-21 拍板：术语由"组"改为"状态"）。
//!
//! VCMI 的动作组是"单段动画"（`ECreatureAnimType`：站立/受击/攻击/行走…），
//! 直接把一个个组摆给用户挑，观感零散、也不像游戏里的生物。这里把组编排成**状态**：
//!
//! - 待机状态：站立 ↔（防御 | 悬停）随机切换，一组播完自然过渡；
//! - 移动状态：**巡逻**——走到一侧（可带纵向偏移 → 斜向）→ 停一下 → 走到另一侧；
//! - 主动攻击状态：攻击（随机方向）→ 顿一下 → 射击/施法（随机方向）→ 顿一下；
//! - 受到攻击状态：站立 → 受伤 ×2~4 → 死亡（播一次停末帧）→ 回到站立。
//!
//! 编排只描述"**播什么、播几圈、停多久、移动到哪**"，不关心取图（高清/1x 由 `getAnimGroup` 决定）；
//! 组缺失（近战没有 SHOOT、原版没有 SPECIAL）由播放器跳过并记忆 —— 见 `CreatureDetail`。
//!
//! ## 移动为什么是"镜像 + 位移 + 缩放"
//!
//! - **向左 = 向右动画的水平镜像**：VCMI 也是这么做的 —— `client/battle/CreatureAnimation.h:75-79`
//!   的 `forward`（朝右）与 `reverse`（"all its frames flipped"）两套动画；
//! - **没有上下/斜向姿态**：H3 生物只有朝右的侧视帧，`MOVING` 是原地迈步（位移由引擎做）。
//!   所以纵向只能"迈步 + 纵向位移 + 纵深缩放"（上=略远略小、下=略近略大）——
//!   这是素材上限，不是实现偷懒；
//! - **显示窗口只有 100×130 源像素**（`CCreaturePic::clipRect`），大生物帧宽接近满窗
//!   （骑士 98/100），故移动状态整体缩到 [`MOVE_ZOOM`] 才腾得出位移空间。

import { ANIM } from "./detailShared";

/** 可切换的动画状态（会话内粘性，默认待机） */
export type AnimState = "idle" | "move" | "attack" | "hurt";

/** 画布姿态：位移 + 水平镜像 + 缩放（全部作用于 canvas 的 CSS transform） */
export interface Motion {
  /** 相对窗口中心的位移（CSS px，右正左负） */
  x: number;
  /** 相对窗口中心的位移（CSS px，下正上负） */
  y: number;
  /** `true` = 水平镜像（向左行走，与 VCMI `reverse` 动画同义） */
  flip: boolean;
  /** 纵深缩放（移动状态基准 [`MOVE_ZOOM`]，上下移动再叠 ±6%） */
  scale: number;
}

/** 初始姿态：居中、不镜像、1:1 */
export const MOTION_HOME: Motion = { x: 0, y: 0, flip: false, scale: 1 };

/** 播放步骤：某动作组循环 `loops` 圈 → 停留 `holdMs` */
export interface AnimStep {
  /** DEF 组号（[`ANIM`]） */
  group: number;
  /** 循环圈数（至少 1） */
  loops: number;
  /** 播完停留（毫秒），期间保持末帧 */
  holdMs: number;
  /** `true` = 播一次停在末帧（死亡类），不循环 */
  once?: boolean;
  /** 本步开始时切换到的姿态（缺省 = 保持不变） */
  motion?: Motion;
  /**
   * `true` = 位移过渡时长**跟随本步实际播放时长**（圈数 × 帧数 × 100ms）。
   * 行走必须用它：脚在原地迈、身体匀速平移，两者同时结束，否则像"滑冰"。
   */
  syncMotion?: boolean;
  /**
   * 期望播放时长（ms）。给了它就由播放器按**实际帧数**换算圈数
   * （`圈数 = wantMs / (帧数 × 100ms)`）—— 各生物 MOVING 帧数从 2 到 12 不等，
   * 固定圈数会让"走得快慢"因生物而异（2 帧的像滑冰、12 帧的像慢动作）。
   */
  wantMs?: number;
}

/** 移动状态的整体缩放：给位移腾空间（大生物帧宽≈满窗，不缩就出框） */
export const MOVE_ZOOM = 0.62;
/** 横向行程（占框宽比例）：0.185 → 180 框下 ±33px，0.62 缩放下不出框 */
const MOVE_X_PCT = 0.185;
/** 纵向行程（占框高比例）：0.17 → 234 框下 ±40px */
const MOVE_Y_PCT = 0.17;
/** 行走速度（px/ms）：30px/s 左右最像"迈步"，快了像滑行、慢了像打太极 */
const WALK_SPEED = 0.03;

const rnd = (n: number) => Math.floor(Math.random() * n);
/** 闭区间随机整数 */
const between = (a: number, b: number) => a + rnd(b - a + 1);
const pick = <T,>(xs: readonly T[]): T => xs[rnd(xs.length)];

/**
 * 方向加权：**正前 50%、上/下各 25%**。
 *
 * 战斗里多数目标在正前方（VCMI `ATTACK_FRONT` 用得最多），均匀随机会让"总是朝上/朝下打"，
 * 观感不自然。
 */
function weightedDir(up: number, front: number, down: number): number {
  const r = Math.random();
  return r < 0.5 ? front : r < 0.75 ? up : down;
}

/** 纵深缩放：y 为负（偏上）→ 略小（远），为正 → 略大（近） */
function depthScale(y: number, boxH: number): number {
  return Math.round(MOVE_ZOOM * (1 + (y / Math.max(1, boxH)) * 0.36) * 1000) / 1000;
}

/**
 * 待机状态：站立（2~4 圈）→ 随机「防御 / 悬停」一次 → 回站立（1~2 圈）。
 *
 * 对应 VCMI `loopPreview` 的观感（`Images.cpp:488-529`），但去掉"随机跳组"的突兀，
 * 改为"站立—反应—站立"的连贯节奏。
 */
export function buildIdle(): AnimStep[] {
  const steps: AnimStep[] = [
    { group: ANIM.HOLDING, loops: between(2, 4), holdMs: 0 },
    { group: pick([ANIM.DEFENCE, ANIM.MOUSEON]), loops: 1, holdMs: 260 },
  ];
  // 12% 插入一次原地"换重心/挪步"（`MOVE_START` 单圈，不位移），避免待机只有两种动作
  if (Math.random() < 0.12) {
    steps.push({ group: ANIM.MOVE_START, loops: 1, holdMs: 80 });
  }
  steps.push({ group: ANIM.HOLDING, loops: between(1, 2), holdMs: 0 });
  return steps;
}

/**
 * **弹窗专用极简待机**：只播 `HOLDING`（1~2 圈）。
 *
 * 右键弹窗是"按住偷看"（`WikiPopup` 在 `mouseup` 即关闭），可见时长通常 0.3~1s：
 * - 播 `DEFENCE/MOUSEON` 要多载 1~2 个组，首开要多付 IPC + webp 解码，收益几乎看不到；
 * - 播移动/攻击/受击更没意义 —— 还会因为巡逻位移把生物推出小画框。
 *
 * 因此弹窗约束为**只有待机**，且只用一个组（HOLDING 本身就是呼吸/摆动动画，足够"活着"）。
 */
export function buildIdleSimple(): AnimStep[] {
  return [{ group: ANIM.HOLDING, loops: between(1, 2), holdMs: 0 }];
}

/**
 * 移动状态（巡逻）：走到一侧 → 停一下 → 走到另一侧，可带纵向偏移形成斜向。
 *
 * 生成一整轮步骤并返回**新的光标**，由调用方（播放器）跨轮保存。
 * 走位越远播放越久（[`WALK_SPEED`] 换算成 `wantMs`），并让位移过渡与播放时长同步。
 */
export function planPatrol(
  cur: Motion,
  boxW: number,
  boxH: number,
): { steps: AnimStep[]; next: Motion } {
  const maxX = Math.max(12, Math.round(boxW * MOVE_X_PCT));
  const maxY = Math.max(12, Math.round(boxH * MOVE_Y_PCT));

  // 10%：原地转身（不移动）—— 打散"左右匀速穿梭"的机械感
  if (Math.random() < 0.1) {
    const flip = !cur.flip;
    const here: Motion = { x: cur.x, y: cur.y, flip, scale: depthScale(cur.y, boxH) };
    return {
      steps: [
        {
          group: flip ? ANIM.TURN_L : ANIM.TURN_R,
          loops: 1,
          holdMs: 80,
          motion: { ...here, scale: MOVE_ZOOM },
        },
        { group: ANIM.HOLDING, loops: 1, holdMs: between(250, 550), motion: here },
      ],
      next: here,
    };
  }

  // 巡逻逻辑：靠近中间 → 随机挑一侧；已在某一侧 → 走到另一侧（"走过去再走回来"）
  const nearCenter = Math.abs(cur.x) < maxX * 0.4;
  const goLeft = nearCenter ? Math.random() < 0.5 : cur.x > 0;
  // 行程偶尔收短一点，避免每次都贴边（显得机械）
  const reach = Math.random() < 0.25 ? 0.65 : 1;
  const tx = Math.round((goLeft ? -maxX : maxX) * reach);
  // 纵向目标：斜向（±0.9×maxY）占多数，纯横向（0 附近）占少数
  const ty =
    Math.random() < 0.25
      ? 0
      : Math.round((Math.random() * 2 - 1) * maxY * 0.9);
  const flip = tx < cur.x;
  const turn = flip !== cur.flip;

  const dist = Math.hypot(tx - cur.x, ty - cur.y);
  // 走多远就播多久：30px/s → 66px 约 2.2s，具体圈数由播放器按实际帧数换算
  const wantMs = Math.round(dist / WALK_SPEED);

  const here: Motion = { x: cur.x, y: cur.y, flip, scale: depthScale(cur.y, boxH) };
  const there: Motion = { x: tx, y: ty, flip, scale: depthScale(ty, boxH) };

  const steps: AnimStep[] = [];
  // 换向：先播转身动画，镜像在转身开始时切换（避免"瞬间翻面"）
  if (turn) {
    steps.push({
      group: flip ? ANIM.TURN_L : ANIM.TURN_R,
      loops: 1,
      holdMs: 60,
      motion: { ...here, flip, scale: MOVE_ZOOM },
    });
  }
  steps.push({ group: ANIM.MOVE_START, loops: 1, holdMs: 0, motion: { ...here, scale: MOVE_ZOOM } });
  // 15%：中途停一下再走（打破"一口气走到底"）
  const midStop = Math.random() < 0.15;
  if (midStop) {
    const mx = Math.round((cur.x + tx) / 2);
    const my = Math.round((cur.y + ty) / 2);
    const mid: Motion = { x: mx, y: my, flip, scale: depthScale(my, boxH) };
    steps.push({
      group: ANIM.MOVING,
      loops: 2,
      holdMs: 0,
      motion: mid,
      syncMotion: true,
      wantMs: Math.round(Math.hypot(mx - cur.x, my - cur.y) / WALK_SPEED),
    });
    steps.push({ group: ANIM.HOLDING, loops: 1, holdMs: between(300, 600), motion: mid });
    steps.push({
      group: ANIM.MOVING,
      loops: 2,
      holdMs: 0,
      motion: there,
      syncMotion: true,
      wantMs: Math.round(Math.hypot(tx - mx, ty - my) / WALK_SPEED),
    });
  } else {
    steps.push({
      group: ANIM.MOVING,
      loops: 2, // 兜底；实际圈数由 `wantMs` 换算
      holdMs: 0,
      motion: there,
      syncMotion: true,
      wantMs,
    });
  }
  steps.push({ group: ANIM.MOVE_END, loops: 1, holdMs: 120, motion: there });
  // 到位后站一会儿（玩家能看清"停在了一侧"）
  steps.push({ group: ANIM.HOLDING, loops: 1, holdMs: between(300, 700), motion: there });

  return { steps, next: there };
}

/**
 * 主动攻击状态：攻击 ×2（加权方向）→ 射击 → 施法。
 *
 * 三段动作语义（实测 + VCMI 枚举）：
 * - 攻击 `ATTACK_UP/FRONT/DOWN`(11/12/13)：原版全族都有，**打两段**让近战也有节奏
 *   （只打一段时，缺射击的近战会剩下两段空停顿）；
 * - 射击 `SHOOT_*`(14/15/16)：远程专用；**原版法师的"施法"就是这一组**（实测 `CMAGE`
 *   只有 14-16，没有 30-32）；
 * - 施法 `CAST_*`(30/31/32)：VCMI 新版组，只有 png/sprites-json 动画的 mod 生物才有。
 *   原版 DEF 里不存在 → 播放器探测失败后跳过（记入 `missingGroups`，不重复付代价）。
 *
 * 因此**不要**把"施法"放回待机：施法是有目标的主动行为；且原版施法已由 SHOOT 覆盖，
 * 待机里另有 `MOUSEON` 承担"悬停反应"。
 */
export function buildAttack(): AnimStep[] {
  return [
    { group: weightedDir(ANIM.ATTACK_UP, ANIM.ATTACK_FRONT, ANIM.ATTACK_DOWN), loops: 1, holdMs: 200 },
    { group: ANIM.HOLDING, loops: 1, holdMs: 150 },
    { group: weightedDir(ANIM.ATTACK_UP, ANIM.ATTACK_FRONT, ANIM.ATTACK_DOWN), loops: 1, holdMs: 220 },
    { group: ANIM.HOLDING, loops: 1, holdMs: 150 },
    { group: weightedDir(ANIM.SHOOT_UP, ANIM.SHOOT_FRONT, ANIM.SHOOT_DOWN), loops: 1, holdMs: 300 },
    { group: ANIM.HOLDING, loops: 1, holdMs: 150 },
    { group: weightedDir(ANIM.CAST_UP, ANIM.CAST_FRONT, ANIM.CAST_DOWN), loops: 1, holdMs: 350 },
  ];
}

/**
 * 受到攻击状态：站立 → 受伤 ×N →（70%）死亡 /（30%）回站立。
 *
 * - 受伤次数：致命路线 2~4 次，非致命路线 1~3 次；
 * - `70%` 死亡：战斗里"被打死"是低概率事件，恒为死亡会让这段循环显得最不自然；
 * - 死亡是**一次性**动画（`once`）：播完停在尸体帧 1.6s，再回到站立。
 */
export function buildHurt(): AnimStep[] {
  const fatal = Math.random() < 0.7;
  const hits = fatal ? between(2, 4) : between(1, 3);
  const steps: AnimStep[] = [{ group: ANIM.HOLDING, loops: 1, holdMs: 120 }];
  for (let i = 0; i < hits; i++) {
    steps.push({ group: ANIM.HITTED, loops: 1, holdMs: 220 });
  }
  if (fatal) {
    steps.push({ group: ANIM.DEATH, loops: 1, holdMs: 1600, once: true });
  } else {
    steps.push({ group: ANIM.HOLDING, loops: between(1, 2), holdMs: 400 });
  }
  return steps;
}

/** 非移动状态的一轮编排（移动状态走 [`planPatrol`]，因为需要跨轮保持光标） */
export function buildSteps(state: Exclude<AnimState, "move">): AnimStep[] {
  switch (state) {
    case "attack":
      return buildAttack();
    case "hurt":
      return buildHurt();
    default:
      return buildIdle();
  }
}
