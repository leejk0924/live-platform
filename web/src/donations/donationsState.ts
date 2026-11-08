import type { Donation, DonationPage } from '../api/types.ts'

export const MAX_SHOWN = 20

// `items` is newest first by id (Snowflake). `total` and `count` are the highest seen: they only
// grow, and a summary can be older than a donation already applied. `listed` is how many
// donations the list accounts for (the loaded page's count plus new ones that arrived live), so
// `count > listed` means some were missed and the list is stale. `fresh` is a donation that just
// arrived live, shown as a notice until it fades.
export type DonationsState = {
  items: readonly Donation[]
  total: number
  count: number
  listed: number
  fresh: Donation | null
}

export type Totals = { total: number; count: number }

export type DonationsAction =
  | { type: 'loaded'; page: DonationPage }
  // A `donation` event: the totals include it.
  | ({ type: 'donated'; donation: Donation } & Totals)
  // A `donations` event.
  | ({ type: 'totals' } & Totals)
  | { type: 'faded'; id: string }

export const initialDonations: DonationsState = { items: [], total: 0, count: 0, listed: 0, fresh: null }

export function donationsReducer(state: DonationsState, action: DonationsAction): DonationsState {
  switch (action.type) {
    case 'loaded':
      return load(state, action.page)
    case 'donated': {
      const totals = higher(state, action)
      if (state.items.some((d) => d.id === action.donation.id)) return { ...state, ...totals }
      return {
        ...totals,
        items: newestFirst([action.donation, ...state.items]),
        listed: state.listed + 1,
        fresh: action.donation,
      }
    }
    case 'totals':
      return { ...state, ...higher(state, action) }
    case 'faded':
      return state.fresh?.id === action.id ? { ...state, fresh: null } : state
  }
}

export const needsReload = (state: DonationsState): boolean => state.count > state.listed

// A donation that arrived live while the page was being read is newer than anything in it; keep
// it, or the reload would hide it until the next one.
function load(state: DonationsState, page: DonationPage): DonationsState {
  const inPage = new Set(page.items.map((d) => d.id))
  const newest = page.items[0]?.id
  const arrived = state.items.filter((d) => !inPage.has(d.id) && (newest === undefined || BigInt(d.id) > BigInt(newest)))
  return {
    ...higher(state, page),
    items: newestFirst([...arrived, ...page.items]),
    listed: page.count + arrived.length,
    fresh: state.fresh,
  }
}

function higher(state: Totals, seen: Totals): Totals {
  return { total: Math.max(state.total, seen.total), count: Math.max(state.count, seen.count) }
}

function newestFirst(items: Donation[]): Donation[] {
  return items.sort((a, b) => (BigInt(a.id) < BigInt(b.id) ? 1 : -1)).slice(0, MAX_SHOWN)
}
