<script setup lang="ts">
import { errorText, t } from '../i18n';
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import StarPet from './StarPet.vue';
import Icon from './Icon.vue';
import { everydayStarPetMoods, getStarPetMood } from '../starpet';
import { dockStarPet, dragStarPet, getStarPetPreferences, onStarPetChanged, onStarPetMoved, onStarPetWindowBlur, setStarPetPreferences, showStarPetMainWindow } from '../starpet-api';
import type { DockEdge, Travel } from '../starpet-locomotion';
import type { StarPetPreferencePatch, StarPetPreferences } from '../starpet-api';
import { emptyGrowth, growthCatalog, levelForXp } from '../starpet-growth';
import type { ActionId, GrowthState } from '../starpet-growth';
import { getStarPetGrowth, interactStarPet, onStarPetGrowthChanged, onStarPetInteraction } from '../starpet-growth-api';
import type { StarPetInteraction } from '../starpet-growth-api';
import { getStarPetSignal, onStarPetSignal, petStatusLabel, petStatusMood } from '../starpet-signal-api';
import type { PetSignal, PetSnapshot } from '../starpet-signal-api';

const preferences = ref<StarPetPreferences>({ enabled: false, mood: 'idle', color: '#FFD66E', size: 100 });
const touchCount = ref(0);
const visible = ref(!document.hidden);
const busy = ref(false);
const loading = ref(true);
const error = ref('');
const growth = ref(emptyGrowth());
const growthBusy = ref(false), growthReady = ref(false), menuOpen = ref(false);
const activeAction = ref<ActionId>('greet'), actionNonce = ref(0), notice = ref('');
const menu = ref<HTMLElement>(), menuButton = ref<HTMLButtonElement>();
const level = computed(() => levelForXp(growth.value.xp));
const petSize = computed(() => 180 * preferences.value.size / 100);
const agentSignal = ref<PetSignal | null>(null);
const mood = computed(() => getStarPetMood(agentSignal.value?.status ? petStatusMood[agentSignal.value.status] : preferences.value.mood));
const agentText = computed(() => agentSignal.value?.message || (agentSignal.value?.action ? t(growthCatalog.actions.find(item => item.id === agentSignal.value?.action)?.message ?? '') : ''));
let unsubscribe: (() => void) | undefined;
let unsubscribeGrowth: (() => void) | undefined;
let unsubscribeBlur: (() => void) | undefined;
let unsubscribeSignal: (() => void) | undefined;
let unsubscribeInteraction: (() => void) | undefined;
let signalTimer: ReturnType<typeof setTimeout> | undefined;
let signalRevision = -1;
let growthLoading = false;
let noticeTimer: ReturnType<typeof setTimeout> | undefined;
let disposed = false, preferenceRevision = 0;
const travel = ref<Travel>({ x: 0, y: 0 }), dock = ref<DockEdge | null>(null);
let unsubscribeMoved: (() => void) | undefined;
let lastMove: { x: number; y: number; at: number } | undefined;
let settleTimer: ReturnType<typeof setTimeout> | undefined, dockTimer: ReturnType<typeof setTimeout> | undefined;
let ignoreMovesUntil = 0, dockRequest = 0;
// Window moves come in physical pixels; about 900 logical px/s counts as a full sprint.
function moved(position: { x: number; y: number }) {
  const now = performance.now();
  if (disposed || now < ignoreMovesUntil) { lastMove = { ...position, at: now }; return; }
  const previous = lastMove;
  lastMove = { ...position, at: now };
  dockRequest++; dock.value = null;
  if (previous) {
    const seconds = Math.max(.008, (now - previous.at) / 1000), scale = window.devicePixelRatio || 1;
    const vx = (position.x - previous.x) / scale / seconds / 900, vy = (position.y - previous.y) / scale / seconds / 900;
    // Smooth jittery move events so the gait does not flicker between directions.
    const clamp = (value: number) => Math.max(-1, Math.min(1, value));
    if (now - previous.at < 250) travel.value = { x: clamp(travel.value.x * .55 + vx * .45), y: clamp(travel.value.y * .55 + vy * .45) };
  }
  clearTimeout(settleTimer); clearTimeout(dockTimer);
  settleTimer = setTimeout(() => { travel.value = { x: 0, y: 0 }; }, 140);
  // The release itself is often swallowed by native dragging, so a short pause ends the drag.
  dockTimer = setTimeout(() => void settle(), 320);
}
async function settle() {
  const request = ++dockRequest;
  try {
    const edge = await dockStarPet();
    if (disposed || request !== dockRequest) return;
    // Our own snap also reports a move; it must not look like another drag.
    ignoreMovesUntil = performance.now() + 200;
    dock.value = edge;
  } catch { /* Docking is decorative: an unknown monitor just leaves the pet where it was dropped. */ }
}
watch(() => preferences.value.size, () => { if (dock.value) void settle(); });
function dismissMenu() { menuOpen.value = false; }
function outsideMenu(event: Event) {
  const target = event.target as Node | null;
  if (menuOpen.value && target && !menu.value?.contains(target) && !menuButton.value?.contains(target)) dismissMenu();
}
function dismissSignal() { clearTimeout(signalTimer); signalTimer = undefined; agentSignal.value = null; }
function acceptInteraction(interaction: StarPetInteraction) {
  if (disposed || interaction.signalRevision < signalRevision) return;
  signalRevision = interaction.signalRevision;
  dismissSignal();
  if (!visible.value || !preferences.value.enabled) return;
  if (interaction.animate) { activeAction.value = interaction.action; actionNonce.value++; }
  const response = t(growthCatalog.actions.find(item => item.id === interaction.action)?.message ?? '');
  notice.value = growth.value.enabled ? `Lv.${level.value} · ${response}` : response;
  clearTimeout(noticeTimer); noticeTimer = setTimeout(() => { notice.value = ''; }, 2600);
}
function acceptSignal(snapshot: PetSnapshot, animate = true) {
  if (disposed || snapshot.revision <= signalRevision) return;
  signalRevision = snapshot.revision;
  dismissSignal();
  if (!visible.value || !snapshot.signal || snapshot.signal.expiresAt <= Date.now()) return;
  agentSignal.value = snapshot.signal;
  notice.value = ''; clearTimeout(noticeTimer);
  if (animate && snapshot.signal.action) { activeAction.value = snapshot.signal.action; actionNonce.value++; }
  signalTimer = setTimeout(dismissSignal, Math.min(60_000, snapshot.signal.expiresAt - Date.now()));
}
async function loadSignal() {
  try {
    unsubscribeSignal = await onStarPetSignal(acceptSignal);
    if (disposed) { unsubscribeSignal(); return; }
    // An initial snapshot restores the caption, but never replays an old action.
    const snapshot = await getStarPetSignal();
    acceptSignal(snapshot, false);
  } catch (reason) { if (!disposed) error.value = t('Agent 提示暂不可用：{error}', { error: errorText(reason) }); }
}
function visibility() { visible.value = !document.hidden; if (!visible.value) { dismissMenu(); clearTimeout(noticeTimer); notice.value = ''; dismissSignal(); } }
function acceptGrowth(value: GrowthState) { if (!disposed && value.revision >= growth.value.revision) growth.value = value; }
async function loadGrowth() {
  if (growthLoading || disposed) return;
  growthLoading = true;
  try {
    if (!unsubscribeGrowth) {
      unsubscribeGrowth = await onStarPetGrowthChanged(acceptGrowth);
      if (disposed) { unsubscribeGrowth(); return; }
    }
    acceptGrowth(await getStarPetGrowth());
    if (!disposed) { growthReady.value = true; if (error.value.startsWith(t('成长状态未能读取：'))) error.value = ''; }
  } catch (reason) { if (!disposed) error.value = t('成长状态未能读取：{error}', { error: errorText(reason) }); }
  finally { growthLoading = false; }
}
onMounted(async () => {
  document.addEventListener('visibilitychange', visibility);
  document.addEventListener('pointerdown', outsideMenu, true);
  document.addEventListener('focusin', outsideMenu);
  window.addEventListener('blur', dismissMenu);
  // DOM blur still dismisses the menu if native event registration is unavailable.
  void onStarPetWindowBlur(dismissMenu).then(stop => {
    if (disposed) stop(); else unsubscribeBlur = stop;
  }).catch(() => {});
  void loadGrowth();
  void loadSignal();
  void onStarPetMoved(moved).then(stop => {
    if (disposed) stop(); else unsubscribeMoved = stop;
  }).catch(() => {});
  void onStarPetInteraction(acceptInteraction).then(stop => {
    if (disposed) stop(); else unsubscribeInteraction = stop;
  }).catch(() => { if (!disposed) error.value = t('动作同步暂不可用，请收起并重新开启星伴后重试。'); });
  try {
    unsubscribe = await onStarPetChanged(value => { preferenceRevision++; preferences.value = value; });
    if (disposed) { unsubscribe(); return; }
    const revision = preferenceRevision;
    const value = await getStarPetPreferences();
    if (!disposed && revision === preferenceRevision) preferences.value = value;
  } catch (reason) { if (!disposed) error.value = errorText(reason); }
  finally { if (!disposed) loading.value = false; }
});
onBeforeUnmount(() => {
  disposed = true; unsubscribeMoved?.(); clearTimeout(settleTimer); clearTimeout(dockTimer); unsubscribe?.(); unsubscribeGrowth?.(); unsubscribeBlur?.(); unsubscribeSignal?.(); unsubscribeInteraction?.(); clearTimeout(noticeTimer); dismissSignal();
  document.removeEventListener('visibilitychange', visibility);
  document.removeEventListener('pointerdown', outsideMenu, true);
  document.removeEventListener('focusin', outsideMenu);
  window.removeEventListener('blur', dismissMenu);
});
function touch(record = true) {
  if (loading.value) return;
  menuOpen.value = false;
  dismissSignal();
  touchCount.value++;
  if (record) void interact('greet', false);
}
async function interact(id: ActionId, animate = true) {
  if (growthBusy.value || loading.value) return;
  closeMenu();
  dismissSignal();
  growthBusy.value = true; error.value = '';
  try {
    const value = await interactStarPet(id, animate);
    if (disposed) return;
    acceptGrowth(value); growthReady.value = true;
    if (!unsubscribeGrowth) void loadGrowth();
    // The validated interaction event animates once, regardless of which window
    // initiated it. The response only updates growth; it never replays an action.
  } catch (reason) { if (!disposed) error.value = errorText(reason); }
  finally { if (!disposed) growthBusy.value = false; }
}
async function toggleMenu() {
  if (menuOpen.value) { closeMenu(); return; }
  menuOpen.value = true;
  if (!growthReady.value || !unsubscribeGrowth) void loadGrowth();
  await nextTick();
  if (!disposed && menuOpen.value) menu.value?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus();
}
function closeMenu() { if (menuOpen.value) { dismissMenu(); menuButton.value?.focus(); } }
async function action(work: () => Promise<unknown>) {
  dismissMenu();
  if (busy.value || loading.value) return;
  busy.value = true; error.value = '';
  try { await work(); } catch (reason) { if (!disposed) error.value = errorText(reason); }
  finally { if (!disposed) busy.value = false; }
}
async function save(preferencesToSave: StarPetPreferencePatch) {
  const revision = preferenceRevision;
  const value = await setStarPetPreferences(preferencesToSave);
  // A newer tray/window event wins over a delayed command response.
  if (!disposed && revision === preferenceRevision) preferences.value = value;
}
function nextMood() {
  dismissSignal();
  const index = everydayStarPetMoods.findIndex(item => item.id === preferences.value.mood);
  return action(() => save({ enabled: true, mood: everydayStarPetMoods[(index + 1) % everydayStarPetMoods.length]!.id }));
}
function hide() { return action(() => save({ enabled: false })); }
function drag(event: PointerEvent) { if (event.button === 0) void action(dragStarPet); }
function grab(event: PointerEvent) {
  if (event.button !== 0 || busy.value || loading.value) return;
  touch(false);
  // Native dragging must start on press; macOS may consume release/click events.
  drag(event);
  // Awarding is deliberately after the native drag call, never on its critical path.
  void interact('greet', false);
}
</script>

<template>
  <main class="desktop-starpet" :class="{ 'menu-open': menuOpen, [`docked-${dock}`]: dock }" :aria-label="t('星伴 · 星芽')" :style="{ '--pet-size': `${petSize}px` }" @keydown.esc.stop="closeMenu">
    <button type="button" class="pet-drag" :disabled="busy || loading" :aria-label="t('拖动星芽')" :title="t('拖动星芽')" @pointerdown.prevent="drag"><Icon name="grip" :size="14" /></button>
    <button type="button" class="pet-body" :disabled="loading" :aria-label="t('摸摸或拖动星芽，现在{mood}', { mood: t(mood.name) })" :title="t('轻点摸摸，按住拖动')" @pointerdown.prevent="grab" @click="$event.detail === 0 && touch()"><StarPet :mood="mood.id" :color="preferences.color" :size="petSize" :reaction="touchCount" :equipment="growth.equipment" :action="activeAction" :action-nonce="actionNonce" :travel="travel" :dock="dock" :animated="preferences.enabled && visible" /></button>
    <span class="pet-caption" role="status" :title="notice || t(mood.name)">{{ notice || `${growth.enabled ? `Lv.${level} · ` : ''}${t(mood.name)}` }}</span>
    <aside v-if="agentSignal" class="pet-agent-message" role="status" :aria-label="t('Agent 星伴提示')"><div class="pet-agent-heading"><strong :title="`${agentSignal.source}${agentSignal.status ? ` · ${t(petStatusLabel[agentSignal.status]!)}` : ''}`">{{ agentSignal.source }}{{ agentSignal.status ? ` · ${t(petStatusLabel[agentSignal.status]!)}` : '' }}</strong><button type="button" :aria-label="t('收起 Agent 提示')" @click="dismissSignal">×</button></div><span v-if="agentText" class="pet-agent-text" tabindex="0" :aria-label="t('Agent 消息正文')">{{ agentText }}</span></aside>
    <nav class="pet-actions" :aria-label="t('星伴操作')"><button type="button" :disabled="busy || loading" :title="t('打开星笺')" :aria-label="t('打开星笺')" @click="action(showStarPetMainWindow)"><Icon name="grid" :size="14" /></button><button type="button" :disabled="busy || loading" :title="t('换个心情')" :aria-label="t('换个心情')" @click="nextMood"><Icon name="spark" :size="14" /></button><button ref="menuButton" type="button" :disabled="loading" :title="t('和星芽互动')" :aria-label="t('和星芽互动')" :aria-expanded="menuOpen" aria-controls="pet-interactions" @click="toggleMenu"><Icon name="star" :size="14" /></button><button type="button" :disabled="busy || loading" :title="t('收起星伴')" :aria-label="t('收起星伴')" @click="hide"><Icon name="close" :size="14" /></button></nav>
    <section v-if="menuOpen" id="pet-interactions" ref="menu" class="pet-menu" :aria-label="t('和星芽互动')">
      <div class="pet-menu-heading"><strong>{{ t('陪星芽玩一会') }}</strong><button type="button" :aria-label="t('关闭互动菜单')" @click="closeMenu">×</button></div>
      <div class="pet-menu-grid"><button v-for="item in growthCatalog.actions" :key="item.id" type="button" :disabled="growthBusy || !growthReady || item.level > level" @click="interact(item.id)">{{ t(item.name) }}<small v-if="item.level > level">Lv.{{ item.level }} {{ t('解锁') }}</small></button></div>
      <button v-if="!growthReady" type="button" class="pet-menu-retry" @click="loadGrowth">{{ t('重新读取成长状态') }}</button>
      <p>{{ growth.enabled ? t('奖励每天各记一次，互动一直有回应。') : t('在偏好设置 → 星伴开启养成。') }}</p>
    </section>
    <p v-if="error" class="pet-error" role="alert">{{ error }}<button type="button" :aria-label="t('关闭错误提示')" @click="error = ''">×</button></p>
  </main>
</template>

<style scoped>
.pet-agent-message{position:absolute;z-index:1;top:2px;left:10px;right:10px;display:flex;flex-direction:column;box-sizing:border-box;max-width:230px;max-height:calc(var(--pet-size)*.4);margin:0 auto;padding:5px 8px;border:1px solid #c6b38a66;border-radius:10px;background:#292838f5;color:#f8ebce;font-size:11px;line-height:1.5;box-shadow:0 4px 12px #0002}.pet-agent-heading{display:flex;align-items:center;gap:4px;flex-shrink:0;min-width:0}.pet-agent-heading strong{flex:1;min-width:0;overflow:hidden;white-space:nowrap;text-overflow:ellipsis;font-size:10px;color:#f2d793}.pet-agent-heading button{flex-shrink:0;width:18px;height:18px;border:0;background:transparent;color:inherit;padding:0;cursor:pointer;font-size:16px;line-height:18px}.pet-agent-text{display:block;min-height:0;overflow:auto;overflow-wrap:anywhere;scrollbar-width:thin}
.pet-menu{position:absolute;z-index:2;bottom:43px;left:8px;right:8px;max-width:230px;max-height:calc(100% - 55px);overflow:auto;box-sizing:border-box;margin:auto;padding:9px;background:#292838f5;color:#f8ebce;border:1px solid #c6b38a66;border-radius:12px;box-shadow:0 5px 15px #0003}.pet-menu-heading{display:flex;align-items:center;justify-content:space-between;gap:6px;margin-bottom:7px;font-size:11px}.pet-menu-heading button{border:0;background:transparent;color:inherit;padding:2px 7px;font-size:16px;cursor:pointer}.pet-menu-grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:5px}.pet-menu-grid button{display:flex;flex-direction:column;justify-content:center;gap:3px;min-height:32px;padding:5px 3px;border:1px solid #f3dfae33;background:#fff1;border-radius:7px;color:inherit;font-size:10px;cursor:pointer}.pet-menu-grid button:disabled{opacity:.45;cursor:default}.pet-menu-grid button:not(:disabled):hover{background:#fff2}.pet-menu-grid small{font-size:8px;color:#e8d5ad}.pet-menu p{font-size:9px;line-height:1.5;margin:8px 0 0;color:#d2c4ac}.pet-menu-retry{color:#f5dca4;background:transparent;border:0;font-size:10px;margin-top:8px;cursor:pointer}.menu-open .pet-actions{opacity:1}.pet-caption{overflow:hidden;white-space:nowrap;text-overflow:ellipsis;max-width:min(190px,calc(100vw - 18px));box-sizing:border-box}.pet-error button{float:right;color:inherit;background:transparent;border:0;font-size:14px;cursor:pointer}
.desktop-starpet{width:100vw;height:100vh;display:flex;flex-direction:column;align-items:center;justify-content:center;position:relative;user-select:none;overflow:hidden;font-family:system-ui,-apple-system,sans-serif}.pet-drag{display:flex;align-items:center;justify-content:center;height:18px;width:46px;flex-shrink:0;padding:0;border:0;border-radius:10px;overflow:hidden;cursor:grab;color:#f3dfae;background:#24232cca;line-height:1;opacity:0;transition:opacity .2s}.pet-drag:active,.pet-body:active{cursor:grabbing}.pet-body{display:flex;align-items:center;justify-content:center;width:calc(var(--pet-size) + 10px);height:var(--pet-size);flex-shrink:0;cursor:grab;touch-action:none;background:transparent;border:0;border-radius:50%;padding:0}.pet-caption{font-size:10px;line-height:1.5;padding:3px 10px;border-radius:10px;color:#fff1cc;background:#24232cde;box-shadow:0 2px 8px #0002;max-width:min(190px,calc(100vw - 18px));text-align:center}.pet-actions{display:flex;align-items:center;justify-content:center;gap:4px;padding:4px;border-radius:13px;margin-top:5px;background:#24232ceb;opacity:0;transition:opacity .2s}.pet-actions button{display:flex;align-items:center;justify-content:center;width:29px;height:25px;color:#f3dfae;border:0;border-radius:9px;background:transparent;cursor:pointer}.pet-actions button:hover{background:#ffffff1a}.desktop-starpet:hover :is(.pet-drag,.pet-actions),.desktop-starpet:focus-within :is(.pet-drag,.pet-actions){opacity:1}.desktop-starpet :focus-visible{outline:2px solid #f2d793;outline-offset:1px}.pet-actions button:disabled{opacity:.5;cursor:wait}.pet-error{position:absolute;z-index:3;inset:8px 8px auto;margin:0;background:#352b32;color:#ffe0d4;border:1px solid #d29090;border-radius:8px;padding:7px;font-size:10px;line-height:1.5;max-height:80px;overflow:auto}.pet-body:hover{filter:brightness(1.03)}/* Docked: push the star against the screen edge it rests on. The window is already flush with that edge. */
.pet-body{transition:transform .35s cubic-bezier(.3,1.35,.5,1)}.docked-left .pet-body{transform:translateX(calc((var(--pet-size) + 10px - 100vw) / 2 - var(--pet-size) * .03))}.docked-right .pet-body{transform:translateX(calc((100vw - var(--pet-size) - 10px) / 2 + var(--pet-size) * .03))}
.docked-top{justify-content:flex-start}.docked-top .pet-body{order:-1;transform:translateY(calc(var(--pet-size) * -.07))}.docked-bottom{justify-content:flex-end}.docked-bottom .pet-body{order:3;transform:translateY(calc(var(--pet-size) * .05))}.docked-bottom .pet-actions{order:2;margin:0 0 5px}
@media(prefers-reduced-motion:reduce){.pet-drag,.pet-actions,.pet-body{transition:none}}
</style>
