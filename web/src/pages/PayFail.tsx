import PayPage from '../donations/PayPage.tsx'

export default function PayFail() {
  return (
    <PayPage title="후원 결제">
      {(_, search) => (
        <p className="error" role="alert">
          결제를 마치지 못했습니다. {search.get('message') ?? '알 수 없는 오류가 발생했습니다.'}
        </p>
      )}
    </PayPage>
  )
}
