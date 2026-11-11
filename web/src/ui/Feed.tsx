import type { ReactNode } from 'react'

const SKELETON_CARDS = [0, 1, 2]

// Placeholder cards the size of real ones, so the grid does not jump when the page arrives.
export function FeedSkeleton() {
  return (
    <>
      <p className="sr-only" role="status">
        불러오는 중…
      </p>
      <ul className="card-grid" aria-hidden="true">
        {SKELETON_CARDS.map((n) => (
          <li key={n} className="card card-skeleton">
            <span className="thumb skeleton" />
            <span className="card-body">
              <span className="skeleton skeleton-line" />
              <span className="skeleton skeleton-line skeleton-short" />
            </span>
          </li>
        ))}
      </ul>
    </>
  )
}

export function EmptyState({ children }: { children: ReactNode }) {
  return (
    <div className="empty">
      <svg className="empty-icon" viewBox="0 0 24 24" aria-hidden="true">
        <rect x="3" y="5" width="18" height="12" rx="2" />
        <path d="M8 21h8M12 17v4" />
      </svg>
      <p>{children}</p>
    </div>
  )
}
