const pad = (n: number) => String(n).padStart(2, '0')

// A comment from today shows its time; an older one, read on an ENDED broadcast days later, also shows its day.
export function commentTime(createdAt: string, now: Date): { text: string; title: string } {
  const at = new Date(createdAt)
  const time = `${pad(at.getHours())}:${pad(at.getMinutes())}`
  const today = at.toDateString() === now.toDateString()
  return {
    text: today ? time : `${at.getMonth() + 1}월 ${at.getDate()}일 ${time}`,
    title: at.toLocaleString('ko-KR'),
  }
}
