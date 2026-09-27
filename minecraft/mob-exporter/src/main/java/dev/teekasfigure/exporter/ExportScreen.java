package dev.teekasfigure.exporter;

import com.google.gson.GsonBuilder;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import net.minecraft.SharedConstants;
import net.minecraft.client.gui.DrawContext;
import net.minecraft.client.gui.screen.Screen;
import net.minecraft.client.gui.widget.ButtonWidget;
import net.minecraft.entity.mob.MobEntity;
import net.minecraft.entity.EntityType;
import net.minecraft.entity.passive.SheepEntity;
import net.minecraft.registry.Registries;
import net.minecraft.text.Text;
import net.minecraft.util.DyeColor;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.LocalDateTime;
import java.time.format.DateTimeFormatter;
import java.util.List;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

public final class ExportScreen extends Screen {
    private final Screen parent;
    private Path folder;
    private static Path lastFolder;
    private CompletableFuture<Path> discovering;
    private CompletableFuture<Void> opening;
    private ButtonWidget topButton,frontButton,backButton;
    private List<EntityType<?>> types;
    private CaptureRenderer renderer;
    private CaptureRenderer.View exportView=CaptureRenderer.View.TOP;
    private final JsonArray rows = new JsonArray(), errors = new JsonArray();
    private int index;
    private boolean running;
    private Text status = UiText.text("teekasfigure.export.ready");
    private CompletableFuture<Void> writing;
    private final ExecutorService io = Executors.newSingleThreadExecutor(r -> { var t=new Thread(r,"TeekasFigure PNG writer"); t.setDaemon(true); return t; });

    public ExportScreen(Screen parent) { super(UiText.text("teekasfigure.export.title")); this.parent=parent; }

    @Override protected void init() {
        topButton=addDrawableChild(ButtonWidget.builder(UiText.text("teekasfigure.export.start_top"), b -> start(CaptureRenderer.View.TOP))
            .dimensions(width/2-110,height/2-31,220,20).build());
        frontButton=addDrawableChild(ButtonWidget.builder(UiText.text("teekasfigure.export.start_front"), b -> start(CaptureRenderer.View.FRONT))
            .dimensions(width/2-110,height/2-5,220,20).build());
        addDrawableChild(ButtonWidget.builder(UiText.text("teekasfigure.export.folder"), b -> openFolder())
            .dimensions(width/2-110,height/2+21,220,20).build());
        addDrawableChild(ButtonWidget.builder(UiText.text("teekasfigure.export.copy"), b -> {
            client.keyboard.setClipboard((folder==null?exportsRoot():folder).toAbsolutePath().toString());status=UiText.text("teekasfigure.export.copied");
        }).dimensions(width/2-110,height/2+47,220,20).build());
        backButton=addDrawableChild(ButtonWidget.builder(UiText.text("teekasfigure.export.back"), b -> close())
            .dimensions(width/2-110,height-28,220,20).build());
        if(folder==null && lastFolder!=null && lastFolder.startsWith(exportsRoot()) && Files.isDirectory(lastFolder))folder=lastFolder;
        if(folder==null && discovering==null){Path root=exportsRoot();discovering=CompletableFuture.supplyAsync(() -> {try{return DesktopIntegration.latestExport(root);}catch(Exception error){throw new RuntimeException(error);}},io);}
    }

    private Path exportsRoot(){return client.runDirectory.toPath().toAbsolutePath().resolve("teekasfigure_exports");}
    private void openFolder() {
        if(opening!=null)return;Path selected=folder==null?exportsRoot():folder;
        opening=CompletableFuture.runAsync(() -> {try{Files.createDirectories(selected);DesktopIntegration.openFolder(selected);}catch(Exception error){throw new RuntimeException(error);}},io);
    }
    @Override public void tick() {
        if(discovering!=null && discovering.isDone()){
            try{Path found=discovering.join();if(folder==null){folder=found;if(!found.equals(exportsRoot())){lastFolder=found;status=UiText.text("teekasfigure.export.last",found.getFileName().toString());}}}
            catch(Exception error){status=UiText.text("teekasfigure.export.folder_failed",error.getCause()==null?error.toString():error.getCause().getMessage());}discovering=null;
        }
        if(opening!=null && opening.isDone()){
            try{opening.join();}catch(Exception error){status=UiText.text("teekasfigure.export.folder_failed",error.getCause()==null?error.toString():error.getCause().getMessage());}opening=null;
        }
        topButton.active=!running && writing==null;
        frontButton.active=!running && writing==null;
        backButton.setMessage(UiText.text(running || writing!=null?"teekasfigure.export.cancel":"teekasfigure.export.back"));
    }

    public void startExport() { start(CaptureRenderer.View.TOP); }
    private void start(CaptureRenderer.View view) {
        if (running || writing!=null) return;
        if (client.world==null) { status=UiText.text("teekasfigure.export.world"); return; }
        try {
            exportView=view;
            folder=exportsRoot().resolve((view==CaptureRenderer.View.TOP?"mobs_top_":"mobs_front_")+
                LocalDateTime.now().format(DateTimeFormatter.ofPattern("yyyyMMdd_HHmmss_SSS")));
            Files.createDirectories(folder.resolve("raw_shapes"));
            lastFolder=folder;
            checkDisk();
            rows.asList().clear(); errors.asList().clear(); index=0;
            types=Registries.ENTITY_TYPE.stream().filter(type -> Registries.ENTITY_TYPE.getId(type).getNamespace().equals("minecraft"))
                .sorted(java.util.Comparator.comparing(type -> Registries.ENTITY_TYPE.getId(type).toString())).toList();
            renderer=new CaptureRenderer(client,512);
            running=true;
            saveManifest(false);
        } catch (Exception error) { fail(error); }
    }

    private void step() {
        if (!running) return;
        if (writing!=null) {
            if (!writing.isDone()) return;
            try { writing.join(); writing=null; } catch(Exception error) { writing=null; fail(error); return; }
        }
        if (index>=types.size()) {
            running=false; renderer.close(); renderer=null;
            writing=CompletableFuture.runAsync(() -> saveManifest(true),io);
            writing.whenComplete((unused,error) -> client.execute(() -> {
                writing=null;
                status=error==null ? UiText.text("teekasfigure.export.done",rows.size(),errors.size()) : UiText.text("teekasfigure.export.failed",error.toString());
            }));
            return;
        }
        var type=types.get(index++);
        var id=Registries.ENTITY_TYPE.getId(type);
        try {
            var entity=type.create(client.world);
            if (!(entity instanceof MobEntity mob)) return;
            mob.setAiDisabled(true); mob.setNoGravity(true); mob.setSilent(true);
            float mobYaw=exportView==CaptureRenderer.View.TOP?180:0;
            mob.setYaw(mobYaw); mob.prevYaw=mobYaw;
            mob.bodyYaw=mobYaw; mob.prevBodyYaw=mobYaw; mob.setHeadYaw(mobYaw); mob.prevHeadYaw=mobYaw;
            mob.setPos(0,0,0); mob.age=0;
            if (mob instanceof SheepEntity sheep) sheep.setColor(DyeColor.WHITE);
            String pose=CapturePoses.forCapture(mob,exportView);CapturePoses.apply(mob,pose);
            // The same pose/model hooks run for captures and real scene entities.
            mob.setCustomName(Text.literal(Participant.PREFIX+"capture:"+pose+":1]"));
            var capture=renderer.capture(mob,exportView);
            String file=id.getNamespace()+"__"+id.getPath().replace('/','_')+".png";
            JsonObject row=new JsonObject();
            row.addProperty("source_name",file); row.addProperty("entity_type",id.toString());
            if(!pose.isEmpty())row.addProperty("pose",pose);
            JsonArray size=new JsonArray(); size.add(capture.span()); size.add(capture.span()); row.add("reference_size_blocks",size);
            row.addProperty("yaw_offset_degrees",mobYaw);
            row.addProperty("depth_blocks",capture.depth()); row.addProperty("min_depth_blocks",capture.minDepth());
            JsonArray pivot=new JsonArray(); pivot.add(capture.centerX()); pivot.add(exportView==CaptureRenderer.View.FRONT?capture.centerOther():0); pivot.add(exportView==CaptureRenderer.View.TOP?capture.centerOther():0); row.add("pivot_blocks",pivot);
            JsonArray center=new JsonArray(); center.add(capture.centerX()); center.add(exportView==CaptureRenderer.View.FRONT?capture.centerOther():0); center.add(exportView==CaptureRenderer.View.TOP?capture.centerOther():0); row.add("capture_center_blocks",center);
            status=Text.literal(id+" · "+index+" / "+types.size());
            writing=CompletableFuture.runAsync(() -> {
                try (var image=capture.image()) {
                    checkDisk();
                    image.writeTo(folder.resolve("raw_shapes").resolve(file));
                    rows.add(row);
                    saveManifest(false);
                } catch(Exception error) { throw new RuntimeException(error); }
            },io);
        } catch(Exception error) {
            JsonObject failed=new JsonObject(); failed.addProperty("entity_type",id.toString()); failed.addProperty("error",error.toString()); errors.add(failed);
            status=UiText.text("teekasfigure.export.skipped",id.toString());
        }
    }

    private void checkDisk() throws java.io.IOException {
        if (Files.getFileStore(folder).getUsableSpace()<256L*1024*1024) throw new java.io.IOException("Less than 256 MiB free disk space");
    }

    private void saveManifest(boolean complete) {
        try {
            JsonObject root=new JsonObject(); root.addProperty("schema_version",1);
            root.addProperty("minecraft_version",SharedConstants.getGameVersion().getName());
            root.addProperty("view",exportView==CaptureRenderer.View.TOP?"top":"front");
            root.addProperty("facing",exportView==CaptureRenderer.View.TOP?"north":"south");
            root.addProperty("resolution",512); root.addProperty("complete",complete);
            root.add("entries",rows); root.add("errors",errors);
            Path temp=folder.resolve("mob_mapping.json.pending");
            Files.writeString(temp,new GsonBuilder().setPrettyPrinting().create().toJson(root));
            Files.move(temp,folder.resolve("mob_mapping.json"),java.nio.file.StandardCopyOption.REPLACE_EXISTING);
        } catch(Exception error) { throw new RuntimeException(error); }
    }

    private void fail(Exception error) {
        running=false; if(renderer!=null) { renderer.close(); renderer=null; }
        status=UiText.text("teekasfigure.export.failed",error.getMessage());
    }

    @Override public void render(DrawContext context,int x,int y,float delta) {
        // Screen.render already draws and blurs the background in Minecraft 1.21.1.
        super.render(context,x,y,delta);
        context.drawCenteredTextWithShadow(textRenderer,title,width/2,20,0xFFFFFF);
        int infoY=43;for(var line:textRenderer.wrapLines(UiText.text("teekasfigure.export.info"),width-24)){context.drawTextWithShadow(textRenderer,line,(width-textRenderer.getWidth(line))/2,infoY,0xCCCCCC);infoY+=10;}
        int statusY=height/2+75;for(var line:textRenderer.wrapLines(status,width-24)){if(statusY>=height-38)break;context.drawTextWithShadow(textRenderer,line,(width-textRenderer.getWidth(line))/2,statusY,0xFFCC80);statusY+=10;}
        context.draw();
        step();
    }

    @Override public void close() { client.setScreen(parent); }
    @Override public void removed() {
        running=false;
        if(renderer!=null) { renderer.close(); renderer=null; }
        io.shutdown(); // Finish the one in-flight PNG, keeping complete=false.
    }
}

