import './globals.css';
import { LanguageProvider } from './language';
export const metadata = {
  metadataBase: new URL('https://neutrondrive.sigmacodez.dev'),
  title: 'NeutronDrive — Your Proton Drive. On Linux.',
  description: 'The independent open-source Proton Drive client for Linux. Sync files, work offline and manage photos. Native Qt interface, MIT licensed.',
  alternates: { canonical: '/' },
  openGraph: { title: 'NeutronDrive — Your Proton Drive. On Linux.', description: 'A home for your cloud. On your Linux desktop.', url: '/', locale: 'en_US', alternateLocale: ['de_DE', 'fr_FR', 'es_ES', 'it_IT'], type: 'website', images: [{url: '/screenshots/files-grid-dark.png', width: 1220, height: 820, alt: 'NeutronDrive desktop interface'}] },
  twitter: {card: 'summary_large_image'},
  icons: { icon: '/icon.svg' }
};
export default function RootLayout({ children }) { return <html lang="en"><body><LanguageProvider>{children}</LanguageProvider></body></html>; }
