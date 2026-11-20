static bool running(){return request&&request->stage>=2&&request->stage<4&&!closing&&uint64_t(std::time(nullptr))<request->packet.deadline&&same_account(request->packet.account);}
static void observe_h5(uint32_t frame,void* packet,void* callback){
    Lock guard;if(!running()||frame!=request->frame||request->callback)return;
    ++request->seen_frame;
    const auto params=small(reinterpret_cast<uintptr_t>(packet)+0x60,131072);
    ++request->seen_params;
    if(!member(params,"url",HISTORY_URL)||!member(params,"scope","subscriptions")||!member(params,"cgi_cmdid","5814",false))return;
    ++request->seen_history;
    std::string body;bool quoted_value=false;
    if(!Json(params).member("req_json",&body,&quoted_value)||!quoted_value||
       !member(body,"BizUserName",request->packet.target)||!member(body,"BizSessionID",std::to_string(request->packet.sequence),false)||
       !member(body,"ActionType","1",false)||!member(body,"PageSize","10",false))return;
    std::string offset;bool is_string=false;const bool has=Json(body).member("Offset",&offset,&is_string);
    if(request->offset.empty()?has:(!has||!is_string||offset!=std::string(request->offset.begin(),request->offset.end())))return;
    if(!callback)return;request->callback=callback;request->stage=3;
}
static void safe_h5(uint32_t frame,void* packet,void* callback){try{observe_h5(frame,packet,callback);}catch(...){}}
static void __fastcall h5_hook(void* a,void* b,uint32_t frame,const SmallString* origin,void* packet,void* callback){
    InterlockedIncrement(&active);
    __try{safe_h5(frame,packet,callback);original_h5(a,b,frame,origin,packet,callback);}
    __finally{InterlockedDecrement(&active);}
}
static void observe_completion(void* holder,void* value){
    Lock guard;if(!running()||!request->callback||holder!=request->callback)return;
    request->callback=nullptr;
    try{JsonProjection projection;projection.value(reinterpret_cast<uintptr_t>(value));request->response=std::move(projection.out);request->stage=4;}
    catch(...){request->error=ERROR_INVALID_DATA;request->stage=4;}
}
static void safe_completion(void* holder,void* value){try{observe_completion(holder,value);}catch(...){}}
static void __fastcall completion_hook(void* holder,void* value){
    InterlockedIncrement(&active);
    __try{safe_completion(holder,value);original_completion(holder,value);}
    __finally{InterlockedDecrement(&active);}
}
static DWORD ensure_hooks(){
    const auto module=GetModuleHandleW(L"wmpf_host_export_x64.dll");
    constexpr unsigned char hash[32]={0x53,0xeb,0xdb,0x04,0x40,0x58,0xb7,0x72,0x47,0x93,0x5d,0xde,0xec,0x3e,0x09,0xf3,0xf3,0x70,0xd0,0x5e,0xa0,0xe2,0xb8,0x08,0x9c,0x58,0xf8,0xe3,0xb3,0x91,0x2d,0x4d};
    constexpr unsigned char new_hash[32]={0xcd,0x91,0x4a,0x30,0xb3,0xdb,0x0b,0x3a,0xc1,0x3e,0xcd,0xe9,0xac,0xac,0xaa,0xf7,0x22,0x00,0x6e,0x8b,0x2d,0xc4,0xfb,0x26,0x1b,0x6d,0x57,0x83,0xeb,0x7a,0xeb,0xef};
    if(core_profile_id==5 ? !fingerprint(module,6189608,new_hash) : core_profile_id!=1||!fingerprint(module,6159920,hash))return ERROR_REVISION_MISMATCH;
    host=reinterpret_cast<uintptr_t>(module);
    const auto init=MH_Initialize();if(init!=MH_OK&&init!=MH_ERROR_ALREADY_INITIALIZED)return ERROR_INVALID_FUNCTION;
    if(!hook_h5){if(MH_CreateHook(reinterpret_cast<void*>(core_address(core_base,0x44c52c0)),reinterpret_cast<void*>(&h5_hook),reinterpret_cast<void**>(&original_h5))!=MH_OK)return ERROR_INVALID_FUNCTION;hook_h5=true;}
    if(!hook_completion){if(MH_CreateHook(reinterpret_cast<void*>(core_address(core_base,0x1f15530)),reinterpret_cast<void*>(&completion_hook),reinterpret_cast<void**>(&original_completion))!=MH_OK)return ERROR_INVALID_FUNCTION;hook_completion=true;}
    for(auto rva:{0x44c52c0,0x1f15530}){const auto status=MH_EnableHook(reinterpret_cast<void*>(core_address(core_base,rva)));if(status!=MH_OK&&status!=MH_ERROR_ENABLED)return ERROR_INVALID_FUNCTION;}
    return 0;
}
static DWORD stop(){
    {Lock guard;request.reset();}
    for(auto entry:{std::pair<uintptr_t,bool>{0x44c52c0,hook_h5},{0x1f15530,hook_completion}}){
        if(!entry.second)continue;const auto s=MH_DisableHook(reinterpret_cast<void*>(core_address(core_base,entry.first)));if(s!=MH_OK&&s!=MH_ERROR_DISABLED)return ERROR_BUSY;
    }
    const auto until=GetTickCount64()+2000;
    while(InterlockedCompareExchange(&active,0,0)&&GetTickCount64()<until)Sleep(1);
    if(active)return ERROR_BUSY;
    if(hook_h5){if(MH_RemoveHook(reinterpret_cast<void*>(core_address(core_base,0x44c52c0)))!=MH_OK)return ERROR_BUSY;hook_h5=false;}
    if(hook_completion){if(MH_RemoveHook(reinterpret_cast<void*>(core_address(core_base,0x1f15530)))!=MH_OK)return ERROR_BUSY;hook_completion=false;}
    return 0;
}
static bool article_url(const std::string& u){
    const std::string prefix="https://mp.weixin.qq.com/s?";
    if(u.size()>4096||u.compare(0,prefix.size(),prefix))return false;
    std::set<std::string> keys;size_t at=prefix.size();
    while(at<u.size()){
        const auto end=u.find('&',at),stop=end==std::string::npos?u.size():end,eq=u.find('=',at);
        if(eq==std::string::npos||eq>=stop||eq+1==stop)return false;
        const auto k=u.substr(at,eq-at),v=u.substr(eq+1,stop-eq-1);
        if(!keys.insert(k).second||!(k=="__biz"||k=="mid"||k=="idx"||k=="sn"))return false;
        for(unsigned char c:v){if(!(std::isalnum(c)||c=='%'||c=='+'||c=='/'||c=='='||c=='_'||c=='-'))return false;
            if((k=="mid"||k=="idx")&&!std::isdigit(c))return false;
            if(k=="sn"&&!std::isxdigit(c))return false;}
        if(k=="sn"&&v.size()!=32)return false;at=stop+1;
    }return keys.size()==4;
}
static std::string script(const Request& r){
    if(r.packet.operation==69){
        const std::string url(r.offset.begin(),r.offset.end()),nonce=r.packet.attempt;
        const auto packet="{\"func\":\"openUrlWithExtraWebview\",\"params\":{\"url\":"+quoted(url)+",\"type\":\"Web\",\"use_system_browser\":false},\"sessionid\":"+quoted(nonce)+",\"__msg_type\":\"call\",\"__callback_id\":"+quoted(nonce)+'}';
        return "(()=>{if(location.protocol!=='weixin:'||location.host!=='resourceid'||location.pathname!=='/SubscriptionDisorder/disorder.html')return;if(typeof MM_CallNative==='function')MM_CallNative('SCENE_FETCHQUEUE',"+quoted(packet)+");})();";
    }
    auto body="{\"BizUserName\":"+quoted(r.packet.target)+",\"ActionType\":1,\"PageSize\":10,\"BizSessionID\":"+std::to_string(r.packet.sequence)+",\"Scene\":207,\"UsePlainTopic\":true,\"PreLoad\":0";
    if(!r.offset.empty())body+=",\"Offset\":"+quoted(std::string(r.offset.begin(),r.offset.end()));body+='}';
    const auto params="{\"cgi_cmdid\":5814,\"url\":"+quoted(HISTORY_URL)+",\"scope\":\"subscriptions\",\"req_json\":"+quoted(body)+'}';
    return "(()=>{if(location.protocol!=='weixin:'||location.host!=='resourceid'||location.pathname!=='/SubscriptionDisorder/disorder.html')return;if(typeof WeixinJSBridge==='object')WeixinJSBridge.invoke('H5ExtTransfer',"+params+",()=>{});})();";
}
static bool execute(const Frame& frame,const char* js){
    __try{reinterpret_cast<Execute>(host_address(0x22c850))(reinterpret_cast<void*>(frame.manager),frame.id,js);return true;}
    __except(EXCEPTION_EXECUTE_HANDLER){return false;}
}
static bool matches(const WercPacket& p){
    return request&&!std::memcmp(p.target,request->packet.target,128)&&!std::memcmp(p.attempt,request->packet.attempt,64)&&
        !std::memcmp(p.digest,request->packet.digest,65)&&p.deadline==request->packet.deadline;
}
static DWORD dispatch(WercPacket& p){
    if(p.operation==63||p.operation==69){
        const auto now=uint64_t(std::time(nullptr));
        if(!ascii_id(p.target,128)||(p.operation==63?std::strncmp(p.target,"gh_",3):std::strcmp(p.target,"article"))||!ascii_id(p.attempt,64)||!digest_valid(p.digest)||p.message_id>8192||!p.sequence||p.sequence>UINT32_MAX||p.deadline<=now||p.deadline>now+45)return ERROR_INVALID_PARAMETER;
        Lock guard;if(request&&request->packet.deadline>now)return ERROR_BUSY;
        request=std::make_unique<Request>();request->packet=p;request->stage=1;return 0;
    }
    // No lock is held across ExecuteJavaScript: the exact callback can be synchronous.
    if(p.operation==65){
        {Lock guard;if(!matches(p))return ERROR_INVALID_DATA;if(request->stage!=1)return ERROR_ALREADY_EXISTS;
            if(request->offset.size()!=request->packet.message_id||!voice_hash(request->offset,p.digest))return ERROR_INVALID_DATA;
            if(!request->offset.empty()&&(!MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,reinterpret_cast<const char*>(request->offset.data()),int(request->offset.size()),nullptr,0)||std::find(request->offset.begin(),request->offset.end(),0)!=request->offset.end()))return ERROR_INVALID_DATA;}
        const auto hooks=ensure_hooks();if(hooks)return hooks;
        Frame frame{};if(!find_frame(&frame))return ERROR_NOT_READY;
        std::string js;
        {Lock guard;if(!matches(p)||closing||uint64_t(std::time(nullptr))>=p.deadline||!same_account(p.account))return ERROR_ACCESS_DENIED;request->frame=frame.id;js=script(*request);if(request->packet.operation==69&&!article_url(std::string(request->offset.begin(),request->offset.end())))return ERROR_INVALID_PARAMETER;request->stage=2;}
        Frame before{};if(!find_frame(&before)||before.manager!=frame.manager||before.id!=frame.id||before.creation!=frame.creation||before.handler!=frame.handler)return ERROR_NOT_READY;
        if(!execute(frame,js.c_str()))return ERROR_INVALID_FUNCTION;
        {Lock guard;if(matches(p)&&request->packet.operation==69){request->response="{\"navigation_issued\":true}";request->stage=4;}}
        return 0;
    }
    Lock guard;if(!matches(p))return ERROR_INVALID_DATA;
    if(p.operation==68){request.reset();return 0;}
    if(uint64_t(std::time(nullptr))>=p.deadline)return ERROR_TIMEOUT;
    if(p.operation==64){
        if(request->stage!=1||p.sequence!=request->offset.size()||!p.present||p.present>1024||p.present>request->packet.message_id-request->offset.size())return ERROR_INVALID_PARAMETER;
        request->offset.insert(request->offset.end(),p.text,p.text+p.present);return 0;
    }
    if(p.operation==66){p.receipt=request->stage;p.dropped=request->error;p.message_id=request->response.size();p.present=request->seen_frame;p.sequence=(uint64_t(request->seen_params)<<32)|request->seen_history;return 0;}
    if(p.operation==67){
        if(request->stage!=4||request->error||p.sequence>=request->response.size())return ERROR_INVALID_PARAMETER;
        p.present=uint32_t(std::min<size_t>(1024,request->response.size()-size_t(p.sequence)));
        SecureZeroMemory(p.text,sizeof(p.text));std::memcpy(p.text,request->response.data()+p.sequence,p.present);p.message_id=request->response.size();return 0;
    }
    return ERROR_INVALID_FUNCTION;
}
} // namespace article_native
