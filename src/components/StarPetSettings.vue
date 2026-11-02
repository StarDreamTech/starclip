<script setup lang="ts">
import { errorText, t } from '../i18n';
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import StarPet from './StarPet.vue';
import StarPetGrowth from './StarPetGrowth.vue';
import { emptyGrowth } from '../starpet-growth';
import type { ActionId } from '../starpet-growth';
import Icon from './Icon.vue';
import UiSelect from './UiSelect.vue';
import UiColor from './UiColor.vue';
import { createStarPetSizeWriter } from '../starpet-size';
import { everydayStarPetMoods, getStarPetMood, starPetColors, starPetGroups, starPetMoods } from '../starpet';
import type { StarPetMoodId } from '../starpet';
import { getStarPetPreferences, onStarPetChanged, setStarPetPreferences, starPetDesktopAvailable } from '../starpet-api';
import type { StarPetPreferences } from '../starpet-api';

const emit = defineEmits<{ busy: [value: boolean] }>();
const props = defineProps<{ disabled?: boolean }>();
const preferences = ref<StarPetPreferences>({ enabled: false, mood: 'idle', color: '#FFD66E', size: 100 });
const selected = ref<StarPetMoodId>('idle');
const group = ref('all');
const showOtherMoods = ref(false);
const playing = ref(false);
const loading = ref(true);
const loaded = ref(false);
const busy = ref(false);
const growthBusy = ref(false);
const growth = ref(emptyGrowth());
const growthPanel = ref<InstanceType<typeof StarPetGrowth>>();
const activeAction = ref<ActionId>('greet');
const actionNonce = ref(0);
const sizeDraft = ref(100);
const sizing = ref(false);
const sizeBlocked = computed(() => props.disabled || loading.value || busy.value || growthBusy.value);
const canRelease = computed(() => loaded.value && !loading.value && !busy.value && !sizing.value && !growthBusy.value
  && selected.value === preferences.value.mood && sizeDraft.value === preferences.value.size && growthPanel.value?.canRelease === true);
defineExpose({ canRelease });
const blocked = computed(() => sizeBlocked.value || sizing.value);
const sizeWriter = createStarPetSizeWriter(async size => {
  const revision = preferenceRevision;
  const saved = await setStarPetPreferences({ size });
  if (!disposed && revision === preferenceRevision) preferences.value = { ...preferences.value, size: saved.size };
  return saved;
});
let sizeRevision = 0;
const reaction = ref(0);
const error = ref('');
const notice = ref('');
const focusedTile = ref<StarPetMoodId | null>(null);
const mood = computed(() => getStarPetMood(selected.value));
const galleryMoods = computed(() => showOtherMoods.value ? starPetMoods : everydayStarPetMoods);
const choices = computed(() => [{ value: 'all', label: t('全部表情 · {count}', { count: galleryMoods.value.length }) }, ...starPetGroups.map(item => ({ value: item.id, label: t(item.name) }))]);
const sizeChoices = computed(() => [{ size: 70, label: t('轻巧') }, { size: 100, label: t('刚刚好') }, { size: 140, label: t('大颗') }]);
const shown = computed(() => galleryMoods.value.filter(item => group.value === 'all' || item.group === group.value));
const tourMoods = computed(() => everydayStarPetMoods.filter(item => group.value === 'all' || item.group === group.value));
let timer: ReturnType<typeof setInterval> | undefined;
let unsubscribe: (() => void) | undefined;
let disposed = false, preferenceRevision = 0;
watch([busy, sizing, growthBusy], () => emit('busy', busy.value || sizing.value || growthBusy.value), { flush: 'sync' });
watch(playing, value => {
  clearInterval(timer);
  if (value) {
    if (!tourMoods.value.some(item => item.id === selected.value)) selected.value = tourMoods.value[0]!.id;
    timer = setInterval(() => {
      const list = tourMoods.value;
      selected.value = list[(list.findIndex(item => item.id === selected.value) + 1) % list.length]!.id;
    }, 2600);
  }
});
watch(group, () => { if (!shown.value.some(item => item.id === selected.value)) selected.value = shown.value[0]!.id; });
onMounted(async () => {
  try {
    unsubscribe = await onStarPetChanged(value => { preferenceRevision++; if (!playing.value && value.mood !== preferences.value.mood) selected.value = value.mood; preferences.value = value; loaded.value = true; if (!sizing.value) sizeDraft.value = value.size; notice.value = ''; error.value = ''; });
    if (disposed) { unsubscribe(); return; }
    const revision = preferenceRevision;
    const value = await getStarPetPreferences();
    if (!disposed) {
      if (revision === preferenceRevision) preferences.value = value;
      selected.value = preferences.value.mood;
      sizeDraft.value = preferences.value.size;
      loaded.value = true;
    }
  } catch (reason) { if (!disposed) error.value = errorText(reason); }
  finally { if (!disposed) loading.value = false; }
});
onBeforeUnmount(() => { disposed = true; unsubscribe?.(); clearInterval(timer); emit('busy', false); });
async function select(id: StarPetMoodId) {
  if (blocked.value) return;
  playing.value = false; selected.value = id; notice.value = ''; error.value = '';
  if (id === preferences.value.mood) return;
  const revision = preferenceRevision;
  busy.value = true;
  try {
    const value = await setStarPetPreferences({ mood: id });
    if (disposed) return;
    if (revision === preferenceRevision) { preferences.value = value; selected.value = value.mood; }
    notice.value = starPetDesktopAvailable ? t('表情已保存，已开启的桌面星芽会同步换心情。') : t('表情已在当前预览生效。');
  } catch (reason) {
    if (!disposed) {
      selected.value = preferences.value.mood;
      error.value = t('表情未能保存：{error}', { error: errorText(reason) });
    }
  } finally { if (!disposed) busy.value = false; }
}
async function save(enabled: boolean) {
  if (blocked.value || !starPetDesktopAvailable) return;
  busy.value = true; error.value = ''; notice.value = ''; playing.value = false;
  try {
    const revision = preferenceRevision;
    const value = await setStarPetPreferences(enabled ? { enabled, mood: selected.value } : { enabled });
    if (disposed) return;
    if (revision === preferenceRevision) preferences.value = value;
    notice.value = preferences.value.enabled ? t('星芽已在桌面等你。按住星芽就可以搬家。') : t('星芽回去休息了，随时可以再叫出来。');
  } catch (reason) { error.value = errorText(reason); }
  finally { busy.value = false; }
}
async function changeColor(color: string) {
  if (blocked.value || color.toUpperCase() === preferences.value.color) return;
  const previous = preferences.value.color, revision = preferenceRevision;
  preferences.value = { ...preferences.value, color: color.toUpperCase() };
  busy.value = true; error.value = ''; notice.value = '';
  try {
    const value = await setStarPetPreferences({ color });
    if (!disposed && revision === preferenceRevision) preferences.value = value;
    if (!disposed) notice.value = starPetDesktopAvailable ? t('新颜色已保存，桌面的星芽也换好衣服了。') : t('颜色已在当前预览生效。');
  } catch (reason) {
    if (!disposed) {
      if (revision === preferenceRevision) preferences.value = { ...preferences.value, color: previous };
      error.value = t('颜色未能保存：{error}', { error: errorText(reason) });
    }
  } finally { busy.value = false; }
}
async function changeSize(size: number) {
  if (sizeBlocked.value || !Number.isInteger(size) || size < 70 || size > 160 || size === sizeDraft.value) return;
  const revision = ++sizeRevision, eventRevision = preferenceRevision;
  sizeDraft.value = size; sizing.value = true;
  error.value = ''; notice.value = '';
  try {
    const saved = await sizeWriter.save(size);
    if (!disposed && revision === sizeRevision && eventRevision === preferenceRevision) preferences.value = saved;
    if (!disposed && revision === sizeRevision) notice.value = starPetDesktopAvailable ? t('大小已保存，桌面的星芽也变好啦。') : t('大小已在当前预览生效。');
  } catch (reason) {
    if (!disposed && revision === sizeRevision) {
      sizeDraft.value = preferences.value.size;
      error.value = t('大小未能保存：{error}', { error: errorText(reason) });
    }
  } finally {
    if (!disposed && revision === sizeRevision) sizing.value = false;
  }
}
function sizeInput(event: Event) { void changeSize(Number((event.target as HTMLInputElement).value)); }
function playAction(id: ActionId) { playing.value = false; activeAction.value = id; actionNonce.value++; }
function touch() { if (!blocked.value) { reaction.value++; void growthPanel.value?.greet(); } }
</script>

<template>
  <div class="starpet-settings">
    <div class="starpet-intro">
      <div class="starpet-stage"><span class="starpet-orbit" aria-hidden="true" /><button type="button" class="starpet-touch" :disabled="blocked" :aria-label="t('摸摸预览里的星芽')" @click="touch"><StarPet :mood="selected" :color="preferences.color" :size="Math.round(144 * sizeDraft / 100)" :reaction="reaction" :equipment="growth.equipment" :action="activeAction" :action-nonce="actionNonce" /></button><span class="starpet-stage-caption">{{ t('轻轻摸一下，或连摸逗逗它') }}</span></div>
      <div class="starpet-intro-copy"><span class="starpet-eyebrow">MEET YOUR LITTLE STAR</span><h3>{{ t('星伴') }} <span>{{ t('· 星芽') }}</span></h3><p>{{ t('会张望、伸懒腰，也会回应你的摸摸。') }}</p><strong>{{ t(mood.name) }}</strong><p class="starpet-description">{{ t(mood.description) }}</p>
        <button type="button" class="button primary" :disabled="blocked || !starPetDesktopAvailable" @click="save(!preferences.enabled)"><Icon :name="preferences.enabled ? 'moon' : 'star'" :size="15" />{{ busy ? t('正在设置…') : preferences.enabled ? t('收起桌面星伴') : t('让星芽住在桌面') }}</button>
        <button v-if="preferences.enabled && selected !== preferences.mood" type="button" class="starpet-apply" :disabled="blocked" @click="save(true)">{{ t('用这个表情陪我') }} <span aria-hidden="true">↗</span></button>
      </div>
    </div>
    <p class="starpet-status" role="status">{{ error || (sizing ? t('正在同步大小…') : notice) || (starPetDesktopAvailable ? t('这里的开关会立即保存。按住桌面星芽就能拖动，轻点会回应你；上方小把手也能拖动。') : t('此处可以预览全部表情；桌面悬浮需要在星笺桌面版开启。')) }}</p>
    <StarPetGrowth ref="growthPanel" :disabled="props.disabled || loading || busy || sizing" :color="preferences.color" :mood="selected" @change="growth = $event" @action="playAction" @busy="growthBusy = $event" />
    <section class="starpet-size" :aria-label="t('星伴大小设置')">
      <div class="starpet-size-heading"><div><label for="starpet-size">{{ t('星芽的大小') }}</label><p>{{ t('小小一颗，或多一点陪伴 · 实时生效，自动保存') }}</p></div><output for="starpet-size">{{ sizeDraft }}<span>%</span></output></div>
      <input id="starpet-size" type="range" min="70" max="160" step="5" :value="sizeDraft" :aria-valuetext="`${sizeDraft}%`" :aria-label="t('星伴大小')" :disabled="sizeBlocked" @input="sizeInput" />
      <div class="starpet-size-choices"><button v-for="choice in sizeChoices" :key="choice.size" type="button" :disabled="sizeBlocked" :aria-pressed="sizeDraft === choice.size" :class="{ chosen: sizeDraft === choice.size }" @click="changeSize(choice.size)">{{ choice.label }} <span>{{ choice.size }}%</span></button></div>
    </section>
    <section class="starpet-colors" :aria-label="t('星芽的颜色')">
      <div class="starpet-color-heading"><div><h4>{{ t('给星芽一点颜色') }}</h4><p>{{ t('轻轻一点，马上换装 · 自动保存') }}</p></div><UiColor :model-value="preferences.color" :label="t('自选星芽颜色')" :disabled="blocked" @update:model-value="changeColor" /></div>
      <div class="starpet-color-choices"><button v-for="color in starPetColors" :key="color.value" type="button" :disabled="blocked" :aria-label="t('星芽颜色：{name}', { name: t(color.name) })" :aria-pressed="preferences.color === color.value" :class="{ chosen: preferences.color === color.value }" @click="changeColor(color.value)"><span class="starpet-color-dot" :style="{ background: color.value }"><Icon v-if="preferences.color === color.value" name="check" :size="13" /></span><span>{{ t(color.name) }}</span></button></div>
    </section>
    <div class="starpet-gallery-heading"><div><h4>{{ t('星芽的 32 种小心情') }}</h4><p>{{ t('点选即保存，已开启的桌面星芽同步换心情；巡游仅预览。') }}</p></div><button type="button" class="starpet-tour" :disabled="blocked" :aria-pressed="playing" @click="playing = !playing"><Icon :name="playing ? 'pause' : 'play'" :size="13" />{{ playing ? t('暂停巡游') : t('表情巡游') }}</button></div>
    <UiSelect v-model="group" :disabled="blocked" :options="choices" :label="t('星伴表情分类')" />
    <label class="starpet-other-moods"><input type="checkbox" :checked="showOtherMoods" :disabled="blocked" @change="showOtherMoods = ($event.target as HTMLInputElement).checked" />{{ t('显示其他心情（需主动选择）') }}</label>
    <div class="starpet-moods" role="group" :aria-label="t('星芽表情')">
      <button v-for="item in shown" :key="item.id" type="button" :disabled="blocked" :class="{ chosen: selected === item.id }" :aria-pressed="selected === item.id" :aria-label="t('选择{name}', { name: t(item.name) })" @click="select(item.id)" @mouseenter="focusedTile = item.id" @mouseleave="focusedTile = null" @focus="focusedTile = item.id" @blur="focusedTile = null"><StarPet :mood="item.id" :color="preferences.color" :size="77" :animated="focusedTile === item.id" /><span>{{ t(item.name) }}</span><Icon v-if="selected === item.id" name="check" :size="11" /></button>
    </div>
    <p class="starpet-footnote">{{ t('原创矢量角色 · 全部本地运行 · 支持系统减少动态效果') }}<br />{{ t('星芽的成长只记录必要计数，不会读取你的剪贴内容，也不需要 AI 服务。') }}</p>
  </div>
</template>

<style scoped>
.starpet-other-moods{display:flex;align-items:center;gap:7px;margin-top:12px;font-size:10px;color:var(--muted);cursor:pointer}.starpet-other-moods input{accent-color:var(--gold)}
.starpet-size{padding:18px 0;border-bottom:1px solid var(--line)}.starpet-size-heading{display:flex;align-items:center;justify-content:space-between;gap:16px}.starpet-size-heading label{font-size:12px}.starpet-size-heading p{font-size:10px;line-height:1.8;color:var(--faint);margin:5px 0 10px}.starpet-size-heading output{font-size:22px;color:var(--gold);font-variant-numeric:tabular-nums}.starpet-size-heading output span{font-size:11px;margin-left:3px}.starpet-size-choices{display:flex;gap:8px;margin-top:9px}.starpet-size-choices button{flex:1;display:flex;justify-content:center;gap:8px;border:1px solid var(--line);border-radius:9px;padding:8px 10px;background:var(--input);font-size:10px;color:var(--muted);cursor:pointer}.starpet-size-choices button span{color:var(--faint);font-variant-numeric:tabular-nums}.starpet-size-choices button.chosen{border-color:var(--gold);color:var(--gold);background:var(--gold-dim)}

.starpet-colors{padding:17px 0;border-bottom:1px solid var(--line)}.starpet-color-heading{display:flex;align-items:center;justify-content:space-between;gap:12px}.starpet-color-heading h4{margin:0 0 5px;font-size:12px;font-weight:500}.starpet-color-heading p{margin:0;font-size:10px;color:var(--faint)}.starpet-color-heading :deep(.ui-color-trigger){min-height:32px}.starpet-color-choices{display:flex;flex-wrap:wrap;gap:7px;margin-top:13px}.starpet-color-choices button{display:flex;align-items:center;gap:6px;border:1px solid var(--line);background:var(--input);border-radius:20px;padding:6px 9px;font-size:10px;color:var(--muted);cursor:pointer}.starpet-color-choices button.chosen{border-color:var(--gold);color:var(--text);background:var(--gold-dim)}.starpet-color-dot{display:grid;place-items:center;width:21px;height:21px;border-radius:50%;color:#4a332f;box-shadow:inset 0 1px 3px #fff9}.starpet-color-choices button:disabled{opacity:.55;cursor:default}

.starpet-touch{display:flex;align-items:center;justify-content:center;position:relative;border:0;padding:0;border-radius:50%;background:transparent;cursor:pointer;touch-action:manipulation;max-width:100%}
.starpet-settings{padding:2px 0 8px}.starpet-intro{display:grid;grid-template-columns:240px minmax(0,1fr);gap:22px;align-items:center}.starpet-stage{position:relative;display:flex;flex-direction:column;align-items:center;justify-content:center;min-height:210px;border:1px solid var(--line);border-radius:20px;background:radial-gradient(ellipse at 50% 45%,var(--gold-dim),transparent 72%),var(--bg);overflow:hidden}.starpet-orbit{position:absolute;width:152px;height:92px;border:1px dashed var(--line);border-radius:50%;transform:rotate(-28deg);pointer-events:none}.starpet-stage-caption{color:var(--faint);font-size:9px;letter-spacing:1px;margin-top:-6px;padding-bottom:12px}.starpet-eyebrow{font-size:8px;letter-spacing:2px;color:var(--faint)}.starpet-intro h3{font-weight:500;font-size:24px;margin:9px 0}.starpet-intro h3 span{color:var(--gold)}.starpet-intro p{font-size:11px;color:var(--muted);line-height:1.8;margin:0 0 11px}.starpet-intro strong{font-size:12px;color:var(--gold);font-weight:500}.starpet-intro .starpet-description{font-size:10px;margin:5px 0 12px;min-height:18px}.starpet-intro .button{font-size:10px;padding:10px 12px}.starpet-apply{display:block;color:var(--gold);font-size:10px;margin-top:10px;padding:3px 0}.starpet-status{font-size:10px;color:var(--muted);line-height:1.8;padding:12px 0 16px;margin:0;border-bottom:1px solid var(--line);min-height:18px}.starpet-gallery-heading{display:flex;align-items:center;justify-content:space-between;margin:18px 0 12px;gap:12px}.starpet-gallery-heading h4{font-size:12px;font-weight:500;margin:0 0 4px}.starpet-gallery-heading p{font-size:10px;color:var(--faint);margin:0}.starpet-tour{display:flex;align-items:center;gap:6px;color:var(--gold);font-size:10px;white-space:nowrap}.starpet-settings>.ui-select{width:170px}.starpet-moods{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:8px;margin-top:12px}.starpet-moods>button{display:flex;position:relative;flex-direction:column;align-items:center;gap:1px;border:1px solid var(--line);border-radius:12px;background:var(--input);color:var(--muted);padding:5px 4px 11px;font-size:10px;cursor:pointer}.starpet-moods>button:hover{background:var(--hover)}.starpet-moods>button.chosen{border-color:var(--gold);background:var(--gold-dim);color:var(--gold)}.starpet-moods>button>svg{position:absolute;top:7px;right:7px}.starpet-footnote{color:var(--faint);font-size:9px;line-height:1.9;margin:17px 0 0}.starpet-settings :focus-visible{outline:2px solid var(--gold);outline-offset:3px}.starpet-settings button:disabled{opacity:.45;cursor:default}@media(max-width:600px){.starpet-intro{grid-template-columns:135px minmax(0,1fr);gap:14px}.starpet-stage{min-height:186px}.starpet-intro h3{font-size:21px}.starpet-eyebrow{letter-spacing:1px;font-size:7px}.starpet-moods{grid-template-columns:repeat(3,minmax(0,1fr))}}
</style>
