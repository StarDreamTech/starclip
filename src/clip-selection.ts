// IDs are independent of the loaded page; only Shift ranges use the current page.
export function selectClip(ids: number[], id: number, page: number[], anchor: number | null, range = false): number[] {
  const start = anchor === null ? -1 : page.indexOf(anchor), end = page.indexOf(id)
  const next = range && start >= 0 && end >= 0
    ? [...new Set([...ids, ...page.slice(Math.min(start, end), Math.max(start, end) + 1)])]
    : ids.includes(id) ? ids.filter(value => value !== id) : [...ids, id]
  if (next.length > 50) throw new Error('每批最多选择 50 张图片。')
  return next
}
