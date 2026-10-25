#include <X11/Xlib.h>
#include <X11/Xatom.h>
#include <string.h>
#include <stdio.h>
int main(int argc,char**argv){Display*d=XOpenDisplay(NULL);if(!d)return 2;Window root=DefaultRootWindow(d),r,p,*children;unsigned int count;int visible=0;
if(!XQueryTree(d,root,&r,&p,&children,&count))return 3;
for(unsigned int i=0;i<count;i++){char*name=NULL;XFetchName(d,children[i],&name);if(name&&strstr(name,"NeutronDrive")){XWindowAttributes a;XGetWindowAttributes(d,children[i],&a);if(a.map_state==IsViewable){visible++;if(argc>1&&!strcmp(argv[1],"close")){XEvent e={0};e.xclient.type=ClientMessage;e.xclient.window=children[i];e.xclient.message_type=XInternAtom(d,"WM_PROTOCOLS",False);e.xclient.format=32;e.xclient.data.l[0]=XInternAtom(d,"WM_DELETE_WINDOW",False);e.xclient.data.l[1]=CurrentTime;XSendEvent(d,children[i],False,NoEventMask,&e);XFlush(d);}}}if(name)XFree(name);}
if(children)XFree(children);XCloseDisplay(d);printf("%d\n",visible);return 0;}
