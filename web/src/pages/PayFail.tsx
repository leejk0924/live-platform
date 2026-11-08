import PayPage from '../donations/PayPage.tsx'
import { failNotice } from '../donations/payResult.ts'

export default function PayFail() {
  return (
    <PayPage title="후원 결제">
      {(_, search) => {
        const { text, code } = failNotice(search)
        return (
          <p className="error" role="alert">
            {text}
            {code && ` (${code})`}
          </p>
        )
      }}
    </PayPage>
  )
}
