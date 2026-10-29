#include <glib-object.h>
#include <gmodule.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
static gpointer emblem_file=NULL;
static int emblem_adds=0,emblem_invalidations=0;
static const char *expected_emblem="neutrondrive-available";
static const char *emblem_uri="file:///tmp/nd-menu-sync/complete";
static char *file_uri(void *file){return g_strdup(file==emblem_file?emblem_uri:file);}
static void add_emblem(void *file,const char *name){g_assert(file==emblem_file);g_assert_cmpstr(name,==,expected_emblem);emblem_adds++;}
#define INFO_STUB(prefix) void prefix##_file_info_add_emblem(void *f,const char *n){add_emblem(f,n);} void prefix##_file_info_invalidate_extension_info(void *f){g_assert(f==emblem_file);emblem_invalidations++;}
INFO_STUB(nautilus) INFO_STUB(nemo) INFO_STUB(caja)
// Isolated fake file-info payloads; menu provider/types/items come from the real host libraries.
char *nautilus_file_info_get_uri(void *file){return file_uri(file);}
char *nemo_file_info_get_uri(void *file){return file_uri(file);}
char *caja_file_info_get_uri(void *file){return file_uri(file);}
char *thunarx_file_info_get_uri(void *file){return file_uri(file);}
static gboolean provider_finalized=FALSE;
static void provider_gone(gpointer data,GObject *object){provider_finalized=TRUE;}
static int activations=0;
// Capture the literal GLib argv instead of launching the user's installed app.
gboolean g_spawn_async(const gchar *cwd,gchar **argv,gchar **env,GSpawnFlags flags,GSpawnChildSetupFunc setup,gpointer data,GPid *pid,GError **error){
 g_assert_cmpstr(argv[0],==,"/usr/bin/neutrondrive");g_assert_cmpstr(argv[1],==,"--file-action");
 const char *modes[]={"pin","evict","share"};g_assert_cmpstr(argv[2],==,modes[activations]);
 g_assert_cmpstr(argv[3],==,"--");g_assert_cmpstr(argv[4],==,"/tmp/nd-menu-sync/a '$(x);.txt");g_assert(argv[5]==NULL);
 g_assert(setup==NULL);activations++;return TRUE;
}
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
int main(int argc,char **argv){
 g_assert(argc==4);
 gboolean legacy=!strcmp(argv[1],"nautilus3");
 const char *prefix=legacy?"nautilus":argv[1];gboolean thunar=!strcmp(prefix,"thunarx"),nautilus=!legacy&&!strcmp(prefix,"nautilus");
 GModule *host=g_module_open(argv[2],G_MODULE_BIND_LAZY);if(!host)g_error("%s",g_module_error());
 GModule *plugin=g_module_open(argv[3],G_MODULE_BIND_LAZY);if(!plugin)g_error("%s",g_module_error());
 GType mt=test_module_get_type();
 if(thunar){GType (*pt)(void)=sym(host,"thunarx_provider_plugin_get_type");GInterfaceInfo ii={thunar_iface,NULL,NULL};g_type_add_interface_static(mt,pt(),&ii);}
 GTypeModule *module=g_object_new(mt,NULL);g_assert(g_type_module_use(module));
 char name[100];snprintf(name,sizeof name,"%s_%s_initialize",thunar?"thunar":prefix,thunar?"extension":"module");
 void (*init)(void*)=sym(plugin,name);init(module);
 snprintf(name,sizeof name,"%s_%s_list_types",thunar?"thunar":prefix,thunar?"extension":"module");
 void (*types_fn)(const GType**,int*)=sym(plugin,name);const GType *types;int count=0;types_fn(&types,&count);g_assert_cmpint(count,==,1);
 gpointer provider=g_object_new(types[0],NULL);
 g_object_weak_ref(provider,provider_gone,NULL);
 snprintf(name,sizeof name,"%s_menu_provider_get_type",prefix);GType (*provider_type)(void)=sym(host,name);
 void **iface=g_type_interface_peek(G_OBJECT_GET_CLASS(provider),provider_type());g_assert(iface);
 GList *selection=g_list_append(NULL,"file:///tmp/nd-menu-sync/a%20%27%24%28x%29%3B.txt");
 GList *(*get2)(void*,GList*)=iface[2];GList *(*get3)(void*,void*,GList*)=iface[2];
 GList *items=nautilus?get2(provider,selection):get3(provider,NULL,selection);g_assert_cmpint(g_list_length(items),==,3);
 for(GList *i=items;i;i=i->next){char *label=NULL;g_object_get(i->data,"label",&label,NULL);g_assert(label&&*label);printf("%s: %s\n",prefix,label);g_free(label);g_signal_emit_by_name(i->data,"activate");}
 g_assert_cmpint(activations,==,3);
 g_list_free_full(items,g_object_unref);
 selection=g_list_append(selection,"file:///tmp/nd-menu-sync/second");
 items=nautilus?get2(provider,selection):get3(provider,NULL,selection);g_assert_cmpint(g_list_length(items),==,2);g_list_free_full(items,g_object_unref);
 selection->data="file:///tmp/outside";
 items=nautilus?get2(provider,selection):get3(provider,NULL,selection);g_assert(items==NULL);
 selection->data="smb://server/tmp/nd-menu-sync/a";
 items=nautilus?get2(provider,selection):get3(provider,NULL,selection);g_assert(items==NULL);
 selection->data="file://remotehost/tmp/nd-menu-sync/a";
 items=nautilus?get2(provider,selection):get3(provider,NULL,selection);g_assert(items==NULL);
 selection->data="file:///tmp/nd-menu-sync/../private";
 items=nautilus?get2(provider,selection):get3(provider,NULL,selection);g_assert(items==NULL);
 g_list_free(selection);
 if(!thunar){
  snprintf(name,sizeof name,"%s_info_provider_get_type",prefix);GType (*info_type)(void)=sym(host,name);
  void **info=g_type_interface_peek(G_OBJECT_GET_CLASS(provider),info_type());g_assert(info);
  int (*update)(void*,void*,void*,void**)=info[2];emblem_file=g_object_new(G_TYPE_OBJECT,NULL);
  const char *index=g_getenv("ND_TEST_AVAILABILITY");g_assert(index);
  g_file_set_contents(index,"{\"version\":1,\"available\":{\"complete\":true}}",-1,NULL);
  g_assert_cmpint(update(provider,emblem_file,NULL,NULL),==,0);g_assert_cmpint(emblem_adds,==,1);
  g_file_set_contents(index,"{\"version\":1,\"available\":{}}",-1,NULL);
  gint64 end=g_get_monotonic_time()+5*G_TIME_SPAN_SECOND;
  while(emblem_invalidations==0&&g_get_monotonic_time()<end){g_main_context_iteration(NULL,FALSE);g_usleep(10000);}
  g_assert_cmpint(emblem_invalidations,>,0);g_assert_cmpint(update(provider,emblem_file,NULL,NULL),==,0);g_assert_cmpint(emblem_adds,==,1);
  char *stat=NULL;g_file_get_contents("/proc/self/stat",&stat,NULL,NULL);
  char **fields=g_strsplit(strrchr(stat,')')+2," ",-1);
  char *progress=g_strdup_printf("{\"version\":1,\"available\":{},\"downloading\":{\"complete\":true},\"owner_pid\":%d,\"owner_start\":\"%s\"}",getpid(),fields[19]);
  g_file_set_contents(index,progress,-1,NULL);g_free(progress);g_strfreev(fields);g_free(stat);
  int before=emblem_invalidations;end=g_get_monotonic_time()+5000000;
  while(emblem_invalidations==before&&g_get_monotonic_time()<end){g_main_context_iteration(NULL,FALSE);g_usleep(10000);}
  expected_emblem="neutrondrive-syncing";
  g_assert_cmpint(emblem_invalidations,>,before);g_assert_cmpint(update(provider,emblem_file,NULL,NULL),==,0);g_assert_cmpint(emblem_adds,==,2);
  g_file_set_contents(index,"{\"version\":1,\"available\":{\"complete\":true}}",-1,NULL);
  before=emblem_invalidations;end=g_get_monotonic_time()+5000000;
  while(emblem_invalidations==before&&g_get_monotonic_time()<end){g_main_context_iteration(NULL,FALSE);g_usleep(10000);}
  expected_emblem="neutrondrive-available";
  g_assert_cmpint(emblem_invalidations,>,before);g_assert_cmpint(update(provider,emblem_file,NULL,NULL),==,0);g_assert_cmpint(emblem_adds,==,3);
  // The host can reuse the same file-info object after a rename.
  emblem_uri="file:///tmp/nd-menu-sync/renamed";
  g_assert_cmpint(update(provider,emblem_file,NULL,NULL),==,0);g_assert_cmpint(emblem_adds,==,3);
  g_file_set_contents(index,"{\"version\":1,\"available\":{\"complete\":true,\"renamed\":true}}",-1,NULL);
  before=emblem_invalidations;end=g_get_monotonic_time()+5000000;
  while(emblem_invalidations==before&&g_get_monotonic_time()<end){g_main_context_iteration(NULL,FALSE);g_usleep(10000);}
  g_assert_cmpint(emblem_invalidations,>,before);g_assert_cmpint(update(provider,emblem_file,NULL,NULL),==,0);g_assert_cmpint(emblem_adds,==,4);
  // Moving outside the sync root must stop tracking the old path.
  emblem_uri="file:///tmp/nd-outside/renamed";
  g_assert_cmpint(update(provider,emblem_file,NULL,NULL),==,0);
  g_file_set_contents(index,"{\"version\":1,\"available\":{}}",-1,NULL);
  before=emblem_invalidations;end=g_get_monotonic_time()+3000000;
  while(g_get_monotonic_time()<end){g_main_context_iteration(NULL,FALSE);g_usleep(10000);}
  g_assert_cmpint(emblem_invalidations,==,before);
  g_object_unref(emblem_file);emblem_file=NULL;
  // Hosts release their provider registry before unloading extensions. Both
  // the volume and emblem timers must allow this finalization to happen now.
  g_object_unref(provider);provider=NULL;g_assert(provider_finalized);
  snprintf(name,sizeof name,"%s_module_shutdown",prefix);void (*shutdown)(void)=sym(plugin,name);shutdown();
  puts("NATIVE_FILE_MANAGER_EMBLEMS_OK");
 }
 if(provider){g_object_unref(provider);g_assert(provider_finalized);}
 puts("NATIVE_FILE_MANAGER_PROVIDER_OK");return 0;
}
