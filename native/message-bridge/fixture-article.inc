static void check_articles(){
    check(article_native::article_url("https://mp.weixin.qq.com/s?__biz=owned&idx=1&mid=123&sn=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),"article public URL allowed");
    check(!article_native::article_url("https://example.com/s?__biz=owned&idx=1&mid=123&sn=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),"article foreign navigation rejected");
    check(!article_native::article_url("https://mp.weixin.qq.com/s?__biz=owned&idx=1&mid=123&sn=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa&key=credential"),"article credential navigation rejected");

    struct Value{uint8_t type;unsigned char padding[7]{};uint64_t data;};
    static_assert(sizeof(Value)==16);
    Value null{0,{},0},boolean{4,{},1},number{6,{},UINT64_MAX};
    article_native::JsonProjection p;p.value(reinterpret_cast<uintptr_t>(&null));check(p.out=="null","article null projection");
    p={};p.value(reinterpret_cast<uintptr_t>(&boolean));check(p.out=="true","article bool projection");
    p={};p.value(reinterpret_cast<uintptr_t>(&number));check(p.out=="18446744073709551615","article preserves 64bit ID");
    SmallString text=voice_view("自动化测试\n",std::strlen("自动化测试\n"));Value string{3,{},reinterpret_cast<uintptr_t>(&text)};
    p={};p.value(reinterpret_cast<uintptr_t>(&string));check(p.out=="\"自动化测试\\u000a\"","article UTF8 and escapes");
    Value values[]={null,boolean,number,string};struct Vector{Value* begin;Value* end;Value* capacity;}v{values,values+4,values+4};Value array{2,{},reinterpret_cast<uintptr_t>(&v)};
    p={};p.value(reinterpret_cast<uintptr_t>(&array));check(p.out.find("[null,true,18446744073709551615,")==0,"article array stride");
    struct Node{Node*left;Node*parent;Node*right;uint8_t color,nil;unsigned char pad[6]{};SmallString key;Value value;};
    static_assert(offsetof(Node,value)==0x40);
    Node head{},node{};head.nil=1;head.left=head.parent=head.right=&node;node.left=node.right=node.parent=&head;node.key=voice_view("owned",5);node.value=array;
    struct Map{Node* head;uint64_t size;}map{&head,1};Value object{1,{},reinterpret_cast<uintptr_t>(&map)};
    p={};p.value(reinterpret_cast<uintptr_t>(&object));check(p.out.find("{\"owned\":[null,true,")==0,"article pinned map layout");
    node.right=&node;bool rejected=false;try{p={};p.value(reinterpret_cast<uintptr_t>(&object));}catch(...){rejected=true;}check(rejected,"article map cycle bounded");
    rejected=false;try{p={};p.value(1);}catch(...){rejected=true;}check(rejected,"article invalid pointer rejected");
    article_native::Request r;strcpy_s(r.packet.target,"gh_owned");r.packet.sequence=123;r.offset={'a','"','b'};const auto js=article_native::script(r);
    check(js.find("H5ExtTransfer")!=std::string::npos&&js.find("SubscriptionDisorder/disorder.html")!=std::string::npos,"article fixed JS guard");
    check(article_native::member("{\"url\":\"owned\",\"x\":1}","url","owned"),"article exact request matching");
    check(!article_native::member("{\"url\":\"owned\",\"url\":\"other\"}","url","owned"),"article duplicate match key rejected");
    WercPacket packet{};packet.operation=63;check(article_native::dispatch(packet)==ERROR_INVALID_PARAMETER,"article invalid begin rejected");
}
