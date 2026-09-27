package dev.teekasfigure.exporter;

import net.minecraft.client.gui.DrawContext;
import net.minecraft.client.gui.screen.Screen;
import net.minecraft.client.gui.widget.ButtonWidget;
import net.minecraft.client.gui.widget.TextFieldWidget;
import net.minecraft.text.Text;
import java.nio.file.*;
import java.util.List;
import java.util.concurrent.*;
import java.util.concurrent.atomic.*;

/** Native selection and background validation; controls describe their actual state. */
public final class PlayerScreen extends Screen {
    private final Screen parent;
    private TextFieldWidget budget,pathField;
    private ButtonWidget chooseButton,startButton,pauseButton,cameraButton,projectionButton,stopButton,optionsButton;
    private Text message=Text.empty();
    private AtomicBoolean cancelled=new AtomicBoolean();
    private final AtomicInteger progress=new AtomicInteger();
    private CompletableFuture<SceneTimeline> loading;
    private CompletableFuture<Path> choosing;
    private String chosenPath="",mobBudget="256";
    private int left,top,panelWidth;
    private static Path lastScene;
    boolean clearOthers;
    public PlayerScreen(Screen parent){super(UiText.text("teekasfigure.player.title"));this.parent=parent;if(lastScene!=null)chosenPath=lastScene.toString();}
    @Override protected void init() {
        if(pathField!=null)chosenPath=pathField.getText();if(budget!=null)mobBudget=budget.getText();
        panelWidth=Math.min(360,width-24);left=(width-panelWidth)/2;top=Math.max(18,(height-190)/2);
        pathField=new TextFieldWidget(textRenderer,left,top+18,panelWidth-96,20,UiText.text("teekasfigure.player.path"));
        pathField.setMaxLength(32768);pathField.setText(chosenPath);addDrawableChild(pathField);
        chooseButton=addDrawableChild(ButtonWidget.builder(UiText.text("teekasfigure.player.choose"),b -> choose()).dimensions(left+panelWidth-90,top+18,90,20).build());
        budget=new TextFieldWidget(textRenderer,left+panelWidth-88,top+46,88,20,UiText.text("teekasfigure.player.budget"));budget.setMaxLength(19);budget.setText(mobBudget);addDrawableChild(budget);
        optionsButton=addDrawableChild(ButtonWidget.builder(UiText.text("teekasfigure.player.options"),b -> client.setScreen(new PlayerOptionsScreen(this))).dimensions(left+92,top+46,panelWidth-190,20).build());
        startButton=addDrawableChild(ButtonWidget.builder(UiText.text("teekasfigure.player.start"),b -> startScene()).dimensions(left,top+74,panelWidth,20).build());
        int half=(panelWidth-6)/2;
        pauseButton=addDrawableChild(ButtonWidget.builder(UiText.text("teekasfigure.player.pause"),b -> serverAction(0)).dimensions(left,top+100,half,20).build());
        cameraButton=addDrawableChild(ButtonWidget.builder(UiText.text("teekasfigure.player.camera"),b -> serverAction(1)).dimensions(left+half+6,top+100,panelWidth-half-6,20).build());
        projectionButton=addDrawableChild(ButtonWidget.builder(UiText.text("teekasfigure.player.ortho"),b -> OrthoCamera.enabled=!OrthoCamera.enabled).dimensions(left,top+126,panelWidth,20).build());
        stopButton=addDrawableChild(ButtonWidget.builder(UiText.text("teekasfigure.player.stop"),b -> {if(loading!=null)cancelled.set(true);serverAction(2);message=UiText.text("teekasfigure.player.stopped");}).dimensions(left,top+152,half,20).build());
        addDrawableChild(ButtonWidget.builder(UiText.text("teekasfigure.export.back"),b -> close()).dimensions(left+half+6,top+152,panelWidth-half-6,20).build());
        updateButtons();
    }
    private void serverAction(int action){var server=client.getServer();if(server==null)return;server.execute(() -> {if(action==0)WorldVideoPlayer.pause(server);else if(action==1)WorldVideoPlayer.resetCamera(server);else WorldVideoPlayer.stop(server);});}
    private void choose() {
        if(loading!=null || choosing!=null)return;
        String title=UiText.text("teekasfigure.player.choose_title").getString();
        Path initial=lastScene==null?client.runDirectory.toPath().resolve("scene.json"):lastScene;
        choosing=CompletableFuture.supplyAsync(() -> DesktopIntegration.chooseScene(title,initial));message=Text.empty();updateButtons();
    }
    private void startScene() {
        if(loading!=null || choosing!=null)return;
        if(client.getServer()==null || client.player==null){message=UiText.text("teekasfigure.player.singleplayer");return;}
        try {
            String entered=pathField.getText().strip();
            if(entered.startsWith("\"") && entered.endsWith("\""))entered=entered.substring(1,entered.length()-1);
            if(entered.isEmpty()){message=UiText.text("teekasfigure.player.idle");return;}
            Path path=DesktopIntegration.normalizeScene(Path.of(entered));
            if(!Files.isRegularFile(path)){message=UiText.text("teekasfigure.player.missing");return;}
            long limit;try{limit=Long.parseLong(budget.getText().strip());if(limit<=0)throw new NumberFormatException();}catch(NumberFormatException error){message=UiText.text("teekasfigure.player.invalid_budget");return;}
            cancelled=new AtomicBoolean();progress.set(0);var token=cancelled;
            lastScene=path;pathField.setText(path.toString());message=Text.empty();
            loading=CompletableFuture.supplyAsync(() -> {try{return new SceneTimeline(path,limit,token,progress);}catch(Exception error){throw new CompletionException(error);}});
        } catch(Exception error){message=UiText.text("teekasfigure.player.failed",reason(error));}
        updateButtons();
    }
    private static String reason(Throwable error){while(error.getCause()!=null)error=error.getCause();return error.getMessage()==null?error.toString():error.getMessage();}
    @Override public void tick() {
        if(choosing!=null && choosing.isDone()) {
            try{Path path=choosing.join();if(path!=null){pathField.setText(path.toString());lastScene=path;message=UiText.text("teekasfigure.player.selected");}}
            catch(Exception error){message=UiText.text("teekasfigure.player.dialog_failed",reason(error));}
            choosing=null;
        }
        if(loading!=null && loading.isDone()) {
            try {
                var scene=loading.join();var server=client.getServer();
                if(server!=null && client.player!=null && !cancelled.get()){
                    var owner=client.player.getUuid();boolean cleanup=clearOthers;OrthoCamera.enabled=true;PlaybackWatchdog.arm(client.runDirectory.toPath());server.execute(() -> WorldVideoPlayer.start(server,owner,scene,cleanup));message=UiText.text("teekasfigure.player.started");
                    loading=null;client.setScreen(null);
                } else {scene.close();message=UiText.text("teekasfigure.player.stopped");}
            } catch(Exception error){message=cancelled.get()?UiText.text("teekasfigure.player.stopped"):UiText.text("teekasfigure.player.failed",reason(error));}
            loading=null;
        }
        updateButtons();
    }
    private void updateButtons() {
        var status=WorldVideoPlayer.status;boolean active=status!=null && status.view()!=null,busy=loading!=null || choosing!=null;
        chooseButton.active=!busy;optionsButton.active=!busy;startButton.active=!busy && client.getServer()!=null;
        pauseButton.active=active;cameraButton.active=active;projectionButton.active=active;stopButton.active=active || loading!=null;
        pauseButton.setMessage(UiText.text(status!=null && status.paused()?"teekasfigure.player.resume":"teekasfigure.player.pause"));
        projectionButton.setMessage(UiText.text(OrthoCamera.enabled?"teekasfigure.player.ortho":"teekasfigure.player.perspective"));
    }
    @Override public void filesDragged(List<Path> paths){if(!paths.isEmpty() && loading==null && choosing==null){pathField.setText(DesktopIntegration.normalizeScene(paths.getFirst()).toString());message=UiText.text("teekasfigure.player.selected");}}
    @Override public boolean shouldPause(){return false;}
    @Override public void render(DrawContext context,int mx,int my,float delta) {
        super.render(context,mx,my,delta);
        context.drawCenteredTextWithShadow(textRenderer,title,width/2,top-18,0xffffff);
        context.drawTextWithShadow(textRenderer,UiText.text("teekasfigure.player.path"),left,top+5,0xffffff);
        context.drawTextWithShadow(textRenderer,UiText.text("teekasfigure.player.budget"),left,top+52,0xffffff);
        var status=WorldVideoPlayer.status;
        Text state=loading!=null?UiText.text("teekasfigure.player.checking",progress.get()):status==null?UiText.text("teekasfigure.player.idle"):
            status.error()!=null?UiText.text("teekasfigure.player.failed",status.error()):UiText.text("teekasfigure.player.status",status.frame()+1,status.total(),status.mobs(),UiText.text(status.loading()?"teekasfigure.player.loading":status.paused()?"teekasfigure.player.paused":"teekasfigure.player.playing").getString());
        int lineY=top+179;for(var line:textRenderer.wrapLines(state,panelWidth)){context.drawTextWithShadow(textRenderer,line,left,lineY,0xffffff);lineY+=10;if(lineY>=height-18)break;}
        if(status!=null && status.view()!=null) {
            var performance=UiText.text("teekasfigure.player.performance",String.format(java.util.Locale.ROOT,"%.1f",status.actualFps()),status.skipped());
            for(var line:textRenderer.wrapLines(performance,panelWidth)){if(lineY>=height-8)break;context.drawTextWithShadow(textRenderer,line,left,lineY,0xaaaaaa);lineY+=10;}
        }
        for(var line:textRenderer.wrapLines(message,panelWidth)){if(lineY>=height-8)break;context.drawTextWithShadow(textRenderer,line,left,lineY,0xffaa55);lineY+=10;}
    }
    @Override public void close(){client.setScreen(OrthoCamera.active()?null:parent);}
    @Override public void removed(){if(loading!=null){cancelled.set(true);loading.thenAccept(scene -> {try{scene.close();}catch(Exception ignored){}});}}
}
