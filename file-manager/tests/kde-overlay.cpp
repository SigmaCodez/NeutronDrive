#include <QApplication>
#include <QPluginLoader>
#include <QFile>
#include <QTimer>
#include <KOverlayIconPlugin>
#include <KFilePlacesModel>
#include <cstdio>
int main(int argc,char**argv){QApplication app(argc,argv);QPluginLoader loader(argv[1]);auto p=qobject_cast<KOverlayIconPlugin*>(loader.instance());if(!p){qCritical("%s",qPrintable(loader.errorString()));return 1;}QUrl url=QUrl::fromLocalFile(argv[2]);if(p->getOverlays(url)!=QStringList{"neutrondrive-available"})return 2;QFile f(argv[3]);if(!f.open(QIODevice::WriteOnly|QIODevice::Truncate))return 3;f.write("{\"version\":1,\"available\":{}}");f.close();QObject::connect(p,&KOverlayIconPlugin::overlaysChanged,&app,[&](const QUrl &u,const QStringList &icons){if(u==url&&icons.isEmpty()){KFilePlacesModel places;bool found=false;for(int r=0;r<places.rowCount();++r)if(places.text(places.index(r,0))=="Proton Drive")found=true;puts(found?"DOLPHIN_OVERLAY_REFRESH_AND_BOOKMARK_OK":"BOOKMARK_MISSING");app.exit(found?0:4);}});QTimer::singleShot(6000,&app,[&]{app.exit(5);});return app.exec();}
