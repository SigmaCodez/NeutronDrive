#include <gio/gio.h>
#include <gmodule.h>
#include <stdio.h>
#include <dlfcn.h>
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

static guint mount_changes=0, monitor_changes=0;
static void changed(GMount *mount,gpointer unused){mount_changes++;}
static void monitor_changed(GVolumeMonitor *monitor,GMount *mount,gpointer target){if(mount==target)monitor_changes++;}

/* An isolated mount uses the real GMount shadow counters and GObject signals,
 * without relying on desktop-specific filtering of temporary kernel mounts. */
typedef struct {GObject parent; GFile *root;} TestMount;
typedef struct {GObjectClass parent;} TestMountClass;
static GFile *test_root(GMount *m){return g_object_ref(((TestMount*)m)->root);}
static void mount_iface(GMountIface *iface){iface->get_root=test_root;}
G_DEFINE_TYPE_WITH_CODE(TestMount,test_mount,G_TYPE_OBJECT,G_IMPLEMENT_INTERFACE(G_TYPE_MOUNT,mount_iface))
static void mount_finalize(GObject *obj){g_clear_object(&((TestMount*)obj)->root);G_OBJECT_CLASS(test_mount_parent_class)->finalize(obj);}
static void test_mount_class_init(TestMountClass *c){G_OBJECT_CLASS(c)->finalize=mount_finalize;}
static void test_mount_init(TestMount *m){}
static GMount *synthetic=NULL;
GList *g_volume_monitor_get_mounts(GVolumeMonitor *monitor){
 if(synthetic)return g_list_append(NULL,g_object_ref(synthetic));
 GList *(*real_mounts)(GVolumeMonitor*)=dlsym(RTLD_NEXT,"g_volume_monitor_get_mounts");
 g_assert(real_mounts);return real_mounts(monitor);
}

int main(int argc,char**argv){
 g_assert(argc==4);
 if(g_getenv("ND_TEST_SYNTHETIC_VOLUME")){
  TestMount *m=g_object_new(test_mount_get_type(),NULL);m->root=g_file_new_for_path(argv[2]);synthetic=G_MOUNT(m);
 }
 GModule *host=g_module_open("libnautilus-extension.so.4",G_MODULE_BIND_LAZY);
 if(!host)host=g_module_open("libnautilus-extension.so.1",G_MODULE_BIND_LAZY);
 g_assert(host);
 GModule *plugin=g_module_open(argv[1],G_MODULE_BIND_LAZY);g_assert(plugin);
 GVolumeMonitor *monitor=g_volume_monitor_get();
 GMount *target=NULL;
 for(int tries=0;tries<100&&!target;tries++){
  GList *mounts=g_volume_monitor_get_mounts(monitor);
  for(GList *l=mounts;l;l=l->next){GFile *f=g_mount_get_root(l->data);char*p=g_file_get_path(f);if(p&&!strcmp(p,argv[2]))target=g_object_ref(l->data);g_free(p);g_object_unref(f);}
  g_list_free_full(mounts,g_object_unref);g_main_context_iteration(NULL,FALSE);g_usleep(20000);
 }
 g_assert(target);g_assert(!g_mount_is_shadowed(target));
 g_signal_connect(target,"changed",G_CALLBACK(changed),NULL);
 g_signal_connect(monitor,"mount-changed",G_CALLBACK(monitor_changed),target);
 GTypeModule *module=g_object_new(test_module_get_type(),NULL);g_assert(g_type_module_use(module));
 void(*init)(void*)=sym(plugin,"nautilus_module_initialize");init(module);
 void(*types_fn)(const GType**,int*)=sym(plugin,"nautilus_module_list_types");const GType*types;int count;types_fn(&types,&count);g_assert_cmpint(count,==,1);
 gpointer provider=g_object_new(types[0],NULL);g_assert(g_mount_is_shadowed(target));
 g_assert_cmpuint(mount_changes,>,0);g_assert_cmpuint(monitor_changes,>,0);
 guint hidden_mount_changes=mount_changes,hidden_monitor_changes=monitor_changes;
 gint64 steady=g_get_monotonic_time()+3*G_TIME_SPAN_SECOND;
 while(g_get_monotonic_time()<steady){g_main_context_iteration(NULL,FALSE);g_usleep(10000);}
 g_assert_cmpuint(mount_changes,==,hidden_mount_changes);g_assert_cmpuint(monitor_changes,==,hidden_monitor_changes);
 g_file_set_contents(argv[3],"{\"mountpoint\":\"/tmp/neutrondrive-no-such-test-mount\"}",-1,NULL);
 gint64 end=g_get_monotonic_time()+5*G_TIME_SPAN_SECOND;
 while(g_mount_is_shadowed(target)&&g_get_monotonic_time()<end){g_main_context_iteration(NULL,FALSE);g_usleep(10000);}
 g_assert(!g_mount_is_shadowed(target));
 g_assert_cmpuint(mount_changes,>,hidden_mount_changes);g_assert_cmpuint(monitor_changes,>,hidden_monitor_changes);
 gchar *config=g_strdup_printf("{\"mountpoint\":\"%s\"}",argv[2]);g_file_set_contents(argv[3],config,-1,NULL);g_free(config);
 end=g_get_monotonic_time()+5*G_TIME_SPAN_SECOND;
 while(!g_mount_is_shadowed(target)&&g_get_monotonic_time()<end){g_main_context_iteration(NULL,FALSE);g_usleep(10000);}
 g_assert(g_mount_is_shadowed(target));hidden_mount_changes=mount_changes;hidden_monitor_changes=monitor_changes;
 void(*shutdown)(void)=sym(plugin,"nautilus_module_shutdown");shutdown();
 g_assert(!g_mount_is_shadowed(target));g_assert_cmpuint(mount_changes,==,hidden_mount_changes);g_assert_cmpuint(monitor_changes,==,hidden_monitor_changes);g_object_unref(provider);g_object_unref(target);g_object_unref(monitor);g_clear_object(&synthetic);puts("GIO_VOLUME_HIDE_CONFIG_RESTORE_AND_QUIET_SHUTDOWN_OK");return 0;
}
