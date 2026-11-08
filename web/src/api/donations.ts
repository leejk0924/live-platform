import { api, unwrap } from './client.ts'

export const donationApi = {
  list: (id: string) => unwrap(api.GET('/api/broadcasts/{id}/donations', { params: { path: { id } } })),
  order: (id: string, amount: number) =>
    unwrap(api.POST('/api/broadcasts/{id}/donations', { params: { path: { id } }, body: { amount } })),
  confirm: (paymentKey: string, orderId: string, amount: number) =>
    unwrap(api.POST('/api/donations/confirm', { body: { paymentKey, orderId, amount } })),
}
