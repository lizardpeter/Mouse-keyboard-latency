//! Standalone native Win32 + ash/Vulkan immediate-present test scene.
//! Runs as a separate child process so Raw Input registration never steals
//! the parent diagnostics application's mouse registration.
//! No hooks, injections, global settings, system files or Vulkan SDK required.
//! NOTE: Vulkan presentation is not a physical photon/scan-out timestamp.
use ash::{vk, Entry, Instance, Device};
use std::{cell::RefCell,ffi::{c_void,CString},io,mem::{size_of,zeroed},
          ptr::{null,null_mut},time::{Duration,Instant}};
type H=*mut c_void;
const WM_INPUT:u32=0xff; const WM_KEYDOWN:u32=0x100; const WM_DESTROY:u32=2;
const WM_CLOSE:u32=0x10; const WM_QUIT:u32=0x12;
const WS_VISIBLE:u32=0x10000000; const WS_CAPTION:u32=0x00c00000;
const WS_SYSMENU:u32=0x00080000; const WS_MINIMIZEBOX:u32=0x00020000;
const PM_REMOVE:u32=1; const RID_INPUT:u32=0x10000003;
const RIDEV_INPUTSINK:u32=0x100;const BUTTON_DOWN:u16=1;
const VK_ESCAPE:usize=27;const VK_SPACE:usize=32;
#[repr(C)]struct WndClass{
    style:u32,proc:Option<unsafe extern "system" fn(H,u32,usize,isize)->isize>,
    cls_extra:i32,wnd_extra:i32,instance:H,icon:H,cursor:H,brush:H,
    menu:*const u16,class:*const u16,
}
#[repr(C)]struct RawDevice{page:u16,usage:u16,flags:u32,hwnd:H}
#[repr(C)]#[derive(Clone,Copy)]struct RawHeader{kind:u32,size:u32,device:H,wp:usize}
#[repr(C)]#[derive(Clone,Copy)]struct RawMouse{
    flags:u16,padding:u16,buttons:u32,raw_buttons:u32,last_x:i32,last_y:i32,extra:u32
}
#[repr(C)]#[derive(Clone,Copy)]union RawData{mouse:RawMouse,padding:[u64;3]}
#[repr(C)]#[derive(Clone,Copy)]struct RawInput{header:RawHeader,data:RawData}
#[repr(C)]#[derive(Default)]struct Point{x:i32,y:i32}
#[repr(C)]#[derive(Clone,Copy)]struct Rect{left:i32,top:i32,right:i32,bottom:i32}
#[repr(C)]struct Msg{hwnd:H,msg:u32,wp:usize,lp:isize,time:u32,point:Point,private:u32}
#[link(name="user32")]unsafe extern "system"{
    fn RegisterClassW(p:*const WndClass)->u16;
    fn CreateWindowExW(ex:u32,class:*const u16,text:*const u16,style:u32,
        x:i32,y:i32,w:i32,h:i32,parent:H,menu:H,instance:H,param:*const c_void)->H;
    fn DefWindowProcW(hwnd:H,msg:u32,wp:usize,lp:isize)->isize;
    fn PeekMessageW(msg:*mut Msg,hwnd:H,min:u32,max:u32,remove:u32)->i32;
    fn TranslateMessage(msg:*const Msg)->i32;
    fn DispatchMessageW(msg:*const Msg)->isize;
    fn DestroyWindow(hwnd:H)->i32;
    fn PostQuitMessage(code:i32);
    fn SetWindowTextW(hwnd:H,text:*const u16)->i32;
    fn RegisterRawInputDevices(devices:*const RawDevice,count:u32,size:u32)->i32;
    fn GetRawInputData(handle:H,command:u32,out:H,bytes:*mut u32,header:u32)->u32;
    fn GetCursorPos(out:*mut Point)->i32;
    fn ScreenToClient(hwnd:H,p:*mut Point)->i32;
    fn GetClientRect(hwnd:H,r:*mut Rect)->i32;
    fn MessageBoxW(hwnd:H,text:*const u16,title:*const u16,flags:u32)->i32;
}
#[link(name="kernel32")]unsafe extern "system" {
    fn GetModuleHandleW(name:*const u16)->H;
    fn QueryPerformanceCounter(out:*mut i64)->i32;
    fn QueryPerformanceFrequency(out:*mut i64)->i32;
}
fn wide(s:&str)->Vec<u16>{s.encode_utf16().chain(std::iter::once(0)).collect()}
fn qpc()->i64{let mut tick=0;unsafe{QueryPerformanceCounter(&mut tick);}tick}
#[derive(Clone,Copy,PartialEq,Eq)]
enum SceneMode{Reaction,Mouse}
#[derive(Clone,Copy)]
enum ReactionPhase{Ready,Waiting{until:Instant},Pending,Visible{tick:i64},Complete{ms:f64},FalseStart}
struct SceneState{
    mode:SceneMode,phase:ReactionPhase,cap:Option<u32>,mouse_events:u64,
    movement_intervals:[f64;2048],interval_index:usize,interval_count:usize,
    last_motion:Option<i64>,frequency:i64,raw_x:i32,raw_y:i32,
    os_x:i32,os_y:i32,trial_count:u64,trial_sum:f64,best:f64,false_starts:u32,
}
impl SceneState{
    fn new()->Self{
        let mut freq=1;unsafe{QueryPerformanceFrequency(&mut freq);}
        Self{mode:SceneMode::Reaction,phase:ReactionPhase::Ready,cap:Some(240),
            mouse_events:0,movement_intervals:[0.0;2048],interval_index:0,
            interval_count:0,last_motion:None,frequency:freq.max(1),
            raw_x:640,raw_y:360,os_x:640,os_y:360,
            trial_count:0,trial_sum:0.0,best:f64::INFINITY,false_starts:0}
    }
    fn click(&mut self,tick:i64){
        if self.mode!=SceneMode::Reaction{return;}
        self.phase=match self.phase{
            ReactionPhase::Waiting{..}|ReactionPhase::Pending=>{
                self.false_starts+=1;ReactionPhase::FalseStart
            }
            ReactionPhase::Visible{tick:signal}=>{
                let ms=tick.saturating_sub(signal) as f64*1000.0/self.frequency as f64;
                self.trial_count+=1;self.trial_sum+=ms;self.best=self.best.min(ms);
                ReactionPhase::Complete{ms}
            }
            phase=>phase
        };
    }
    fn arm(&mut self){
        if self.mode!=SceneMode::Reaction{return;}
        let delay=1400+(qpc().unsigned_abs()%2500);
        self.phase=ReactionPhase::Waiting{until:Instant::now()+Duration::from_millis(delay)};
    }
    fn motion(&mut self,tick:i64,dx:i32,dy:i32,absolute:bool){
        if dx==0&&dy==0{return;}
        self.mouse_events+=1;
        if let Some(last)=self.last_motion{
            let gap=tick.saturating_sub(last) as f64*1000.0/self.frequency as f64;
            if gap>0.0{
                self.movement_intervals[self.interval_index]=gap;
                self.interval_index=(self.interval_index+1)%2048;
                self.interval_count=(self.interval_count+1).min(2048);
            }
        }
        self.last_motion=Some(tick);
        if !absolute{
            self.raw_x=self.raw_x.saturating_add(dx).clamp(4,1275);
            self.raw_y=self.raw_y.saturating_add(dy).clamp(4,715);
        }
    }
    fn estimated_polling_hz(&self)->Option<f64>{
        if self.interval_count<6{return None;}
        let mut v=self.movement_intervals[..self.interval_count].to_vec();
        v.sort_by(|a,b|a.total_cmp(b));
        Some(1000.0/v[v.len()/2])
    }
}
thread_local!{static SCENE:RefCell<SceneState>=RefCell::new(SceneState::new());}
unsafe extern "system" fn wnd_proc(hwnd:H,msg:u32,wp:usize,lp:isize)->isize {
    match msg {
        WM_CLOSE=>{DestroyWindow(hwnd);return 0;}
        WM_DESTROY=>{PostQuitMessage(0);return 0;}
        WM_KEYDOWN=>{
            SCENE.with(|cell|{
                let mut scene=cell.borrow_mut();
                match wp{
                    VK_SPACE=>scene.arm(),
                    0x4d=>scene.mode=SceneMode::Mouse,
                    0x52=>scene.mode=SceneMode::Reaction,
                    0x70=>scene.cap=Some(60),
                    0x71=>scene.cap=Some(120),
                    0x72=>scene.cap=Some(144),
                    0x73=>scene.cap=Some(240),
                    0x74=>scene.cap=Some(360),
                    0x75=>scene.cap=Some(500),
                    0x76=>scene.cap=Some(1000),
                    0x77=>scene.cap=None,
                    _=>{}
                }
            });
            if wp==VK_ESCAPE{DestroyWindow(hwnd);}
            return 0;
        }
        WM_INPUT=>{
            let mut data:RawInput=zeroed();let mut bytes=size_of::<RawInput>() as u32;
            let actual=GetRawInputData(lp as H,RID_INPUT,
                (&mut data as *mut RawInput).cast(),&mut bytes,size_of::<RawHeader>() as u32);
            if actual!=u32::MAX && data.header.kind==0 && actual as usize>=48{
                let raw=data.data.mouse;let tick=qpc();
                let mut p=Point::default();
                if GetCursorPos(&mut p)!=0 {ScreenToClient(hwnd,&mut p);}
                SCENE.with(|c|{
                    let mut s=c.borrow_mut();
                    s.os_x=p.x;s.os_y=p.y;
                    s.motion(tick,raw.last_x,raw.last_y,raw.flags&1!=0);
                    if raw.buttons as u16&BUTTON_DOWN!=0{s.click(tick);}
                });
            }
            return DefWindowProcW(hwnd,msg,wp,lp);
        }
        _=>{}
    }
    DefWindowProcW(hwnd,msg,wp,lp)
}
struct Renderer{
    instance:Instance,device:Device,surface_loader:ash::khr::surface::Instance,
    swap_loader:ash::khr::swapchain::Device,
    surface:vk::SurfaceKHR,swap:vk::SwapchainKHR,
    queue:vk::Queue,pool:vk::CommandPool,cmd:vk::CommandBuffer,
    renderpass:vk::RenderPass,views:Vec<vk::ImageView>,fb:Vec<vk::Framebuffer>,
    available:vk::Semaphore,finished:Vec<vk::Semaphore>,fence:vk::Fence,
    extent:vk::Extent2D,
}
impl Renderer{
    unsafe fn new(hwnd:H,instance_handle:H,entry:&Entry)->Result<Self,Box<dyn std::error::Error>>{
        let name=CString::new("Input Timing Lab - Immediate Vulkan Scene")?;
        let app=vk::ApplicationInfo::default().application_name(&name)
            .api_version(vk::API_VERSION_1_0);
        let exts=[ash::khr::surface::NAME.as_ptr(),ash::khr::win32_surface::NAME.as_ptr()];
        let create=vk::InstanceCreateInfo::default().application_info(&app)
            .enabled_extension_names(&exts);
        let instance=entry.create_instance(&create,None)?;
        let win32=ash::khr::win32_surface::Instance::new(entry,&instance);
        let surface_loader=ash::khr::surface::Instance::new(entry,&instance);
        let info=vk::Win32SurfaceCreateInfoKHR::default()
            .hinstance(instance_handle as isize).hwnd(hwnd as isize);
        let surface=win32.create_win32_surface(&info,None)?;
        let mut chosen=None;
        for phys in instance.enumerate_physical_devices()?{
            for (i,props) in instance.get_physical_device_queue_family_properties(phys).iter().enumerate(){
                if props.queue_flags.contains(vk::QueueFlags::GRAPHICS)
                   && surface_loader.get_physical_device_surface_support(phys,i as u32,surface)?{
                    chosen=Some((phys,i as u32));break;
                }
            }
            if chosen.is_some(){break;}
        }
        let (phys,family)=chosen.ok_or("No Vulkan graphics queue supports this Win32 surface")?;
        let priority=[1.0];
        let queue_info=[vk::DeviceQueueCreateInfo::default()
            .queue_family_index(family).queue_priorities(&priority)];
        let extensions=[ash::khr::swapchain::NAME.as_ptr()];
        let dev_info=vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_info).enabled_extension_names(&extensions);
        let device=instance.create_device(phys,&dev_info,None)?;
        let queue=device.get_device_queue(family,0);
        let swap_loader=ash::khr::swapchain::Device::new(&instance,&device);
        let caps=surface_loader.get_physical_device_surface_capabilities(phys,surface)?;
        let formats=surface_loader.get_physical_device_surface_formats(phys,surface)?;
        let modes=surface_loader.get_physical_device_surface_present_modes(phys,surface)?;
        if !modes.contains(&vk::PresentModeKHR::IMMEDIATE){
            return Err("Vulkan IMMEDIATE present mode is unsupported on this display. Refusing silent V-sync fallback.".into());
        }
        if !caps.supported_usage_flags.contains(vk::ImageUsageFlags::COLOR_ATTACHMENT){
            return Err("Vulkan surface does not support color attachment images".into());
        }
        let format=*formats.iter().find(|f|f.format==vk::Format::B8G8R8A8_UNORM)
            .or_else(||formats.first()).ok_or("No Vulkan surface formats")?;
        let extent=caps.current_extent;
        if extent.width==0||extent.height==0 {
            return Err("Vulkan surface has zero extent".into());
        }
        let count=(caps.min_image_count+1).max(2);
        let count=if caps.max_image_count>0{count.min(caps.max_image_count)}else{count};
        let alpha=[vk::CompositeAlphaFlagsKHR::OPAQUE,vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED,
            vk::CompositeAlphaFlagsKHR::POST_MULTIPLIED,vk::CompositeAlphaFlagsKHR::INHERIT]
            .into_iter().find(|f|caps.supported_composite_alpha.contains(*f))
            .ok_or("No supported Vulkan composite alpha")?;
        let swap_info=vk::SwapchainCreateInfoKHR::default()
            .surface(surface).min_image_count(count)
            .image_color_space(format.color_space).image_format(format.format)
            .image_extent(extent).image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(caps.current_transform).composite_alpha(alpha)
            .present_mode(vk::PresentModeKHR::IMMEDIATE).clipped(true);
        let swap=swap_loader.create_swapchain(&swap_info,None)?;
        let images=swap_loader.get_swapchain_images(swap)?;
        let attachment=[vk::AttachmentDescription::default().format(format.format)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::PRESENT_SRC_KHR)];
        let ref_color=[vk::AttachmentReference{
            attachment:0,layout:vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL}];
        let subpass=[vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&ref_color)];
        let dependencies=[vk::SubpassDependency::default()
            .src_subpass(vk::SUBPASS_EXTERNAL).dst_subpass(0)
            .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)];
        let rp=vk::RenderPassCreateInfo::default().attachments(&attachment)
            .subpasses(&subpass).dependencies(&dependencies);
        let renderpass=device.create_render_pass(&rp,None)?;
        let mut views=Vec::new();let mut fb=Vec::new();
        for image in images{
            let view_info=vk::ImageViewCreateInfo::default().image(image)
                .view_type(vk::ImageViewType::TYPE_2D).format(format.format)
                .subresource_range(vk::ImageSubresourceRange{
                    aspect_mask:vk::ImageAspectFlags::COLOR,
                    base_mip_level:0,level_count:1,base_array_layer:0,layer_count:1
                });
            let view=device.create_image_view(&view_info,None)?;
            let attachments=[view];
            let fb_info=vk::FramebufferCreateInfo::default()
                .render_pass(renderpass).attachments(&attachments)
                .width(extent.width).height(extent.height).layers(1);
            fb.push(device.create_framebuffer(&fb_info,None)?);views.push(view);
        }
        let pool_info=vk::CommandPoolCreateInfo::default().queue_family_index(family)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        let pool=device.create_command_pool(&pool_info,None)?;
        let buffers=device.allocate_command_buffers(
            &vk::CommandBufferAllocateInfo::default()
                .command_pool(pool).level(vk::CommandBufferLevel::PRIMARY).command_buffer_count(1))?;
        let cmd=buffers[0];
        let sem_info=vk::SemaphoreCreateInfo::default();
        let available=device.create_semaphore(&sem_info,None)?;
        let mut finished=Vec::new();
        for _ in &fb{finished.push(device.create_semaphore(&sem_info,None)?);}
        let fence=device.create_fence(&vk::FenceCreateInfo::default()
            .flags(vk::FenceCreateFlags::SIGNALED),None)?;
        Ok(Self{instance,device,surface_loader,swap_loader,surface,swap,queue,
            pool,cmd,renderpass,views,fb,available,finished,fence,extent})
    }
    unsafe fn frame(&self,mode:SceneMode,phase:ReactionPhase,raw:(i32,i32),
                    cursor:(i32,i32))->Result<(),Box<dyn std::error::Error>> {
        self.device.wait_for_fences(&[self.fence],true,u64::MAX)?;
        let (idx,_)=self.swap_loader.acquire_next_image(self.swap,u64::MAX,
            self.available,vk::Fence::null())?;
        self.device.reset_fences(&[self.fence])?;
        self.device.reset_command_buffer(self.cmd,vk::CommandBufferResetFlags::empty())?;
        self.device.begin_command_buffer(self.cmd,
            &vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT))?;
        let bg=match mode{
            SceneMode::Mouse=>[0.025,0.035,0.065,1.0],
            SceneMode::Reaction=>match phase{
                ReactionPhase::Waiting{..}|ReactionPhase::Pending=>[0.45,0.13,0.13,1.0],
                ReactionPhase::Visible{..}=>[0.03,0.68,0.22,1.0],
                ReactionPhase::FalseStart=>[0.60,0.30,0.03,1.0],
                _=>[0.05,0.09,0.15,1.0]
            }
        };
        let clear=[vk::ClearValue{color:vk::ClearColorValue{float32:bg}}];
        let begin=vk::RenderPassBeginInfo::default().render_pass(self.renderpass)
            .framebuffer(self.fb[idx as usize])
            .render_area(vk::Rect2D{offset:vk::Offset2D{x:0,y:0},extent:self.extent})
            .clear_values(&clear);
        self.device.cmd_begin_render_pass(self.cmd,&begin,vk::SubpassContents::INLINE);
        // Vulkan native colored target/cursors: no DX/GDI intermediary or shader compilation.
        if mode==SceneMode::Mouse{
            let w=self.extent.width as i32;let h=self.extent.height as i32;
            for (x,y,rgba,size) in [
                (w/2,h/2,[0.28,0.29,0.36,1.0],35),
                (raw.0*w/1280,raw.1*h/720,[0.20,0.92,0.96,1.0],13),
                (cursor.0,cursor.1,[1.0,0.69,0.17,1.0],9)
            ]{
                let x=(x-size/2).clamp(0,w-1);let y=(y-size/2).clamp(0,h-1);
                let extent=vk::Extent2D{
                    width:(size as u32).min((w-x) as u32),
                    height:(size as u32).min((h-y) as u32)};
                let attachments=[vk::ClearAttachment{
                    aspect_mask:vk::ImageAspectFlags::COLOR,color_attachment:0,
                    clear_value:vk::ClearValue{color:vk::ClearColorValue{float32:rgba}}
                }];
                let rect=[vk::ClearRect{
                    rect:vk::Rect2D{offset:vk::Offset2D{x,y},extent},
                    base_array_layer:0,layer_count:1
                }];
                self.device.cmd_clear_attachments(self.cmd,&attachments,&rect);
            }
        } else {
            let w=self.extent.width as i32;let h=self.extent.height as i32;
            let size=if matches!(phase,ReactionPhase::Visible{..}){120}else{52};
            let attachments=[vk::ClearAttachment{
                aspect_mask:vk::ImageAspectFlags::COLOR,color_attachment:0,
                clear_value:vk::ClearValue{color:vk::ClearColorValue{
                    float32:[0.95,0.96,0.98,1.0]}}
            }];
            let rect=[vk::ClearRect{rect:vk::Rect2D{
                offset:vk::Offset2D{x:w/2-size/2,y:h/2-size/2},
                extent:vk::Extent2D{width:size as u32,height:size as u32}},
                base_array_layer:0,layer_count:1}];
            self.device.cmd_clear_attachments(self.cmd,&attachments,&rect);
        }
        self.device.cmd_end_render_pass(self.cmd);
        self.device.end_command_buffer(self.cmd)?;
        let waits=[self.available];let stages=[vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
        let cmds=[self.cmd];let signals=[self.finished[idx as usize]];
        let submit=[vk::SubmitInfo::default().wait_semaphores(&waits)
            .wait_dst_stage_mask(&stages).command_buffers(&cmds).signal_semaphores(&signals)];
        self.device.queue_submit(self.queue,&submit,self.fence)?;
        let swaps=[self.swap];let indices=[idx];
        let present=vk::PresentInfoKHR::default().wait_semaphores(&signals)
            .swapchains(&swaps).image_indices(&indices);
        self.swap_loader.queue_present(self.queue,&present)?;
        Ok(())
    }
    unsafe fn shutdown(self){
        let _=self.device.device_wait_idle();
        self.device.destroy_fence(self.fence,None);
        self.device.destroy_semaphore(self.available,None);
        for s in &self.finished{self.device.destroy_semaphore(*s,None);}
        self.device.destroy_command_pool(self.pool,None);
        for fb in &self.fb{self.device.destroy_framebuffer(*fb,None);}
        for v in &self.views{self.device.destroy_image_view(*v,None);}
        self.device.destroy_render_pass(self.renderpass,None);
        self.swap_loader.destroy_swapchain(self.swap,None);
        self.device.destroy_device(None);
        self.surface_loader.destroy_surface(self.surface,None);
        self.instance.destroy_instance(None);
    }
}
pub fn run()->Result<(),Box<dyn std::error::Error>>{
    unsafe{
        if size_of::<usize>()!=8||size_of::<RawInput>()!=48{
            return Err("Windows x64 ABI required".into());
        }
        let cls=wide("InputTimingLabVulkanScene");
        let inst=GetModuleHandleW(null());
        let wclass=WndClass{style:0,proc:Some(wnd_proc),cls_extra:0,wnd_extra:0,
            instance:inst,icon:null_mut(),cursor:null_mut(),brush:6 as H,
            menu:null(),class:cls.as_ptr()};
        if RegisterClassW(&wclass)==0{return Err(io::Error::last_os_error().into());}
        let title=wide("Vulkan Input Lab | R=Reaction M=Mouse | SPACE=Start | F1-F8=FPS | ESC=Close");
        let hwnd=CreateWindowExW(0,cls.as_ptr(),title.as_ptr(),
            WS_VISIBLE|WS_CAPTION|WS_SYSMENU|WS_MINIMIZEBOX,
            120,80,1280,760,null_mut(),null_mut(),inst,null());
        if hwnd.is_null(){return Err(io::Error::last_os_error().into());}
        let raw=[RawDevice{page:1,usage:2,flags:RIDEV_INPUTSINK,hwnd}];
        if RegisterRawInputDevices(raw.as_ptr(),1,size_of::<RawDevice>() as u32)==0{
            return Err(io::Error::last_os_error().into());
        }
        let entry=Entry::load()?;
        let renderer=Renderer::new(hwnd,inst,&entry)?;
        let mut last_title=Instant::now();
        let mut frame_count=0u64;
        let mut should_close=false;
        let mut message:Msg=zeroed();
        while !should_close{
            while PeekMessageW(&mut message,null_mut(),0,0,PM_REMOVE)!=0{
                if message.msg==WM_QUIT{should_close=true;break;}
                TranslateMessage(&message);DispatchMessageW(&message);
            }
            if should_close{break;}
            let frame_start=Instant::now();
            let (mode,phase,raw,cursor,cap)=SCENE.with(|cell|{
                let mut scene=cell.borrow_mut();
                if let ReactionPhase::Waiting{until}=scene.phase {
                    if Instant::now()>=until{scene.phase=ReactionPhase::Pending;}
                }
                (scene.mode,scene.phase,(scene.raw_x,scene.raw_y),
                 (scene.os_x,scene.os_y),scene.cap)
            });
            if let Err(e)=renderer.frame(mode,phase,raw,cursor){
                let msg=wide(&format!("Vulkan scene stopped: {e}"));
                MessageBoxW(hwnd,msg.as_ptr(),wide("Vulkan error").as_ptr(),0x10);
                break;
            }
            if matches!(phase,ReactionPhase::Pending){
                SCENE.with(|cell|{
                    let mut scene=cell.borrow_mut();
                    if matches!(scene.phase,ReactionPhase::Pending){
                        scene.phase=ReactionPhase::Visible{tick:qpc()};
                    }
                });
            }
            frame_count+=1;
            if last_title.elapsed()>=Duration::from_secs(1){
                let rate=frame_count as f64/last_title.elapsed().as_secs_f64();
                let title=SCENE.with(|cell|{
                    let scene=cell.borrow();
                    let caps=scene.cap.map_or("Unlimited".to_owned(),|v|format!("{v}"));
                    match scene.mode{
                        SceneMode::Reaction=>{
                            let phase=match scene.phase{
                                ReactionPhase::Ready=>"SPACE to start".into(),
                                ReactionPhase::Waiting{..}=>"WAIT (red)".into(),
                                ReactionPhase::Pending=>"Preparing frame".into(),
                                ReactionPhase::Visible{..}=>"CLICK GREEN!".into(),
                                ReactionPhase::FalseStart=>"FALSE START — SPACE retry".into(),
                                ReactionPhase::Complete{ms}=>format!("Reaction {ms:.1}ms — SPACE retry")
                            };
                            format!("Vulkan REACTION | {phase} | {rate:.0} FPS (cap {caps}) | best {:.1}ms | {} trials | M=Mouse R=Reaction F1-F8=cap",
                                scene.best,scene.trial_count)
                        },
                        SceneMode::Mouse=>format!(
                            "Vulkan MOUSE | {rate:.0} FPS (cap {caps}) | raw movement {} | median ~{} Hz | CYAN=raw counts ORANGE=Windows cursor | R=Reaction F1-F8=cap",
                            scene.mouse_events,scene.estimated_polling_hz()
                                .map_or_else(||"—".into(),|v|format!("{v:.0}")))
                    }
                });
                SetWindowTextW(hwnd,wide(&title).as_ptr());
                last_title=Instant::now();frame_count=0;
            }
            if let Some(fps)=cap {
                let desired=Duration::from_secs_f64(1.0/fps as f64);
                let end=frame_start+desired;
                let remaining=end.saturating_duration_since(Instant::now());
                if remaining>Duration::from_millis(2){
                    std::thread::sleep(remaining-Duration::from_millis(1));
                }
                while Instant::now()<end{std::hint::spin_loop();}
            }
        }
        renderer.shutdown();
        DestroyWindow(hwnd);
        Ok(())
    }
}