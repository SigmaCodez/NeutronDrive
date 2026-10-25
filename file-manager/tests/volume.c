#include <gio/gio.h>
#include <gmodule.h>
#include <stdio.h>
typedef struct {GTypeModule parent;} TestModule;
typedef struct {GTypeModuleClass parent;} TestModuleClass;
G_DEFINE_TYPE(TestModule,test_module,G_TYPE_TYPE_MODULE)
static gboolean load(GTypeModule *m){return TRUE;}
static void unload(GTypeModule *m){}
static void get_property(GObject *obj,guint id,GValue *value,GParamSpec *pspec){g_value_set_boolean(value,TRUE);}
static void set_property(GObject *obj,guint id,const GValue *value,GParamSpec *pspec){}
static void test_module_class_init(TestModuleClass *c){
 G_OBJECT_CLASS(c)->get_property=get_property;G_OBJECT_CLASS(c)->set_property=set_property;
 g_object_class_install_property(G_OBJECT_CLASS(c),1,g_param_spec_boolean("resident","Resident","Resident",TRUE,G_PARAM_READWRITE));G_TYPE_MODULE_CLASS(c)->load=load;G_TYPE_MODULE_CLASS(c)->unload=unload;}
static void test_module_init(TestModule *m){}
static void thunar_iface(void *iface,void *unused){
 void **slots=(void**)iface;
 slots[4]=g_type_module_register_type; slots[5]=g_type_module_add_interface;
}
static void *sym(GModule *m,const char *name){void *v=NULL;g_assert(g_module_symbol(m,name,&v));return v;}

int main(int argc,char**argv){
 g_assert(argc==4);
 GModule *host=g_module_open("libnautilus-extension.so.4",G_MODULE_BIND_LAZY);g_assert(host);
 GModule *plugin=g_module_open(argv[1],G_MODULE_BIND_LAZY);g_assert(plugin);
 GVolumeMonitor *monitor=g_volume_monitor_get();
 GMount *target=NULL;
 for(int tries=0;tries<100&&!target;tries++){
  GList *mounts=g_volume_monitor_get_mounts(monitor);
  for(GList *l=mounts;l;l=l->next){GFile *f=g_mount_get_root(l->data);char*p=g_file_get_path(f);if(p&&!strcmp(p,argv[2]))target=g_object_ref(l->data);g_free(p);g_object_unref(f);}
  g_list_free_full(mounts,g_object_unref);g_main_context_iteration(NULL,FALSE);g_usleep(20000);
 }
 g_assert(target);g_assert(!g_mount_is_shadowed(target));
 GTypeModule *module=g_object_new(test_module_get_type(),NULL);g_assert(g_type_module_use(module));
 void(*init)(void*)=sym(plugin,"nautilus_module_initialize");init(module);
 void(*types_fn)(const GType**,int*)=sym(plugin,"nautilus_module_list_types");const GType*types;int count;types_fn(&types,&count);g_assert_cmpint(count,==,1);
 gpointer provider=g_object_new(types[0],NULL);g_assert(g_mount_is_shadowed(target));
 g_file_set_contents(argv[3],"{\"mountpoint\":\"/tmp/neutrondrive-no-such-test-mount\"}",-1,NULL);
 gint64 end=g_get_monotonic_time()+5*G_TIME_SPAN_SECOND;
 while(g_mount_is_shadowed(target)&&g_get_monotonic_time()<end){g_main_context_iteration(NULL,FALSE);g_usleep(10000);}
 g_assert(!g_mount_is_shadowed(target));
 void(*shutdown)(void)=sym(plugin,"nautilus_module_shutdown");shutdown();g_object_unref(provider);g_object_unref(target);g_object_unref(monitor);puts("GIO_VOLUME_HIDE_AND_CONFIG_CHANGE_RESTORE_OK");return 0;
}
