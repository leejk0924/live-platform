import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { createBrowserRouter, RouterProvider } from 'react-router'
import './index.css'
import { AuthProvider } from './auth/AuthProvider.tsx'
import Layout from './Layout.tsx'
import BroadcastDetail from './pages/BroadcastDetail.tsx'
import BroadcastEdit from './pages/BroadcastEdit.tsx'
import BroadcastNew from './pages/BroadcastNew.tsx'
import Home from './pages/Home.tsx'
import Login from './pages/Login.tsx'
import PayCheckout from './pages/PayCheckout.tsx'
import PayFail from './pages/PayFail.tsx'
import PaySuccess from './pages/PaySuccess.tsx'
import RecordingDetail from './pages/RecordingDetail.tsx'
import RecordingList from './pages/RecordingList.tsx'
import Signup from './pages/Signup.tsx'
import NotFound from './pages/NotFound.tsx'

const router = createBrowserRouter([
  {
    element: <Layout />,
    children: [
      { index: true, element: <Home /> },
      { path: 'broadcasts/new', element: <BroadcastNew /> },
      { path: 'broadcasts/:id', element: <BroadcastDetail /> },
      { path: 'broadcasts/:id/edit', element: <BroadcastEdit /> },
      { path: 'recordings', element: <RecordingList /> },
      { path: 'recordings/:id', element: <RecordingDetail /> },
      { path: 'pay/checkout', element: <PayCheckout /> },
      { path: 'pay/success', element: <PaySuccess /> },
      { path: 'pay/fail', element: <PayFail /> },
      { path: 'login', element: <Login /> },
      { path: 'signup', element: <Signup /> },
      { path: '*', element: <NotFound /> },
    ],
  },
])

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <AuthProvider>
      <RouterProvider router={router} />
    </AuthProvider>
  </StrictMode>,
)
