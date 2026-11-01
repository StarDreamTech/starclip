<script setup lang="ts">
withDefaults(defineProps<{ name: string; size?: number }>(), { size: 18 })
const paths: Record<string, string> = {
  image: 'M3 4h18v16H3z M3 16l5-6 5 5 3-3 5 5 M17 8h.01',
  file: 'M6 3h8l4 4v14H6z M14 3v4h4 M9 12h6 M9 16h6',
  star: 'm12 3 2.8 5.7 6.2.9-4.5 4.4 1.1 6.2L12 17.3l-5.6 2.9 1.1-6.2L3 9.6l6.2-.9L12 3Z', spark: 'M12 2 9.5 9.5 2 12l7.5 2.5L12 22l2.5-7.5L22 12l-7.5-2.5L12 2Z',
  chart: 'M4 20V11 M10 20V4 M16 20v-7 M3 20h18',
  grid: 'M3 3h7v7H3z M14 3h7v7h-7z M3 14h7v7H3z M14 14h7v7h-7z', list: 'M8 5h13 M8 12h13 M8 19h13 M3 5h.01 M3 12h.01 M3 19h.01', clock: 'M12 8v4l3 2 M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z', search: 'm21 21-4.4-4.4 M19 11a8 8 0 1 1-16 0 8 8 0 0 1 16 0Z',
  link: 'M10 13a5 5 0 0 0 7 .1l3-3a5 5 0 0 0-7-7l-1.7 1.7 M14 11a5 5 0 0 0-7-.1l-3 3a5 5 0 0 0 7 7l1.7-1.7', code: 'm8 7-5 5 5 5 m8-10 5 5-5 5 m-3-14-2 18', text: 'M4 5h16 M12 5v15 M8 20h8', color: 'M12 3s-7 7.3-7 12a7 7 0 0 0 14 0c0-4.7-7-12-7-12Z',
  folder: 'M3 7V5a2 2 0 0 1 2-2h5l2 3h7a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7Z', plus: 'M12 5v14 M5 12h14', copy: 'M9 9h12v12H9z M15 9V3H3v12h6', check: 'm5 12 4 4L19 6', close: 'm6 6 12 12 M18 6 6 18', sliders: 'M4 6h4 M12 6h8 M4 18h8 M16 18h4 M8 3v6 M16 15v6 M4 12h12 M20 12h.01 M16 9v6',
  pause: 'M8 5v14 M16 5v14', play: 'm8 4 13 8-13 8V4Z', shield: 'm12 3 8 3v6c0 5-8 9-8 9s-8-4-8-9V6l8-3Z m-4 9 3 3 5-6', chevron: 'm9 5 7 7-7 7', arrow: 'M5 12h14 m-5-5 5 5-5 5', down: 'M12 3v12 m-5-5 5 5 5-5 M4 16v5h16v-5', upload: 'M12 16V4 m-5 5 5-5 5 5 M4 17v4h16v-4',
  trash: 'M3 6h18 M9 6V3h6v3 M5 6l1 15h12l1-15 M10 10v7 M14 10v7', edit: 'm15 5 4 4 M4 20l4-1L21 6l-4-4L4 15v5Z', moon: 'M20.9 13.3A9 9 0 0 1 10.7 3.1 9 9 0 1 0 20.9 13.3Z', sun: 'M12 3V1 M12 23v-2 M3 12H1 M23 12h-2 M4 4l2 2 M18 18l2 2 M4 20l2-2 M18 6l2-2 M17 12a5 5 0 1 1-10 0 5 5 0 0 1 10 0Z', alert: 'm12 3 10 18H2L12 3Z M12 9v5 M12 17h.01', more: 'M5 12h.01 M12 12h.01 M19 12h.01',
}
</script>
<template><svg :width="size" :height="size" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path :d="paths[name] || paths.spark" /></svg></template>
