import type { FormEvent } from 'react'
import type { BroadcastInput } from '../api/types.ts'

type Props = {
  initial?: BroadcastInput
  submitLabel: string
  pending: boolean
  error: string | null
  onSubmit: (input: BroadcastInput) => void
}

export default function BroadcastForm({ initial, submitLabel, pending, error, onSubmit }: Props) {
  function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const data = new FormData(event.currentTarget)
    onSubmit({ title: String(data.get('title')), description: String(data.get('description')) })
  }

  return (
    <form className="stack" onSubmit={handleSubmit}>
      <label>
        제목
        <input name="title" required maxLength={100} defaultValue={initial?.title} />
      </label>
      <label>
        설명
        <textarea name="description" rows={6} maxLength={2000} defaultValue={initial?.description} />
      </label>
      {error && (
        <p className="error" role="alert">
          {error}
        </p>
      )}
      <button type="submit" disabled={pending}>
        {submitLabel}
      </button>
    </form>
  )
}
