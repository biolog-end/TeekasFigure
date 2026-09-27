package dev.teekasfigure.smoke;
import net.fabricmc.api.ClientModInitializer;
import net.minecraft.client.MinecraftClient;
import net.minecraft.client.gui.screen.TitleScreen;
import net.minecraft.client.gui.screen.GameMenuScreen;
import net.minecraft.world.level.LevelInfo;
import net.minecraft.world.GameMode;
import net.minecraft.world.GameRules;
import net.minecraft.world.Difficulty;
import net.minecraft.resource.DataConfiguration;
import net.minecraft.world.gen.GeneratorOptions;
import net.minecraft.world.gen.WorldPresets;
import dev.teekasfigure.exporter.ExportScreen;
import java.nio.file.*;
import java.time.*;
public final class SmokeMod implements ClientModInitializer {
    private int phase;
    private long start=System.currentTimeMillis();
    @Override public void onInitializeClient(){
        var worker=new Thread(() -> {while(true){try{Thread.sleep(500);MinecraftClient.getInstance().execute(() -> step(MinecraftClient.getInstance()));}catch(InterruptedException e){return;}}},"TF smoke driver");worker.setDaemon(true);worker.start();
    }
    private void step(MinecraftClient client){
        try{
            if(System.currentTimeMillis()-start>240_000)throw new Exception("Smoke test timed out, phase "+phase);
            if(phase==0 && client.currentScreen instanceof TitleScreen){
                phase=1;
                var rules=new GameRules();rules.get(GameRules.DO_MOB_SPAWNING).set(false,null);
                client.createIntegratedServerLoader().createAndStart("tf-smoke-"+System.currentTimeMillis(),new LevelInfo("TF smoke",GameMode.CREATIVE,false,Difficulty.PEACEFUL,true,rules,DataConfiguration.SAFE_MODE),new GeneratorOptions(42,false,false),WorldPresets::createDemoOptions,client.currentScreen);
            } else if(phase==1 && client.world!=null && client.player!=null && client.currentScreen==null){
                phase=2;var screen=new ExportScreen(new GameMenuScreen(true));client.setScreen(screen);
                // The same public action as the export button, after screen initialization.
                screen.startExport();
            } else if(phase==2){
                Path root=client.runDirectory.toPath().resolve("teekasfigure_exports");
                if(Files.isDirectory(root))try(var paths=Files.list(root)){
                    for(Path folder:paths.toList()){
                        Path manifest=folder.resolve("mob_mapping.json");
                        if(Files.exists(manifest) && com.google.gson.JsonParser.parseString(Files.readString(manifest)).getAsJsonObject().get("complete").getAsBoolean()){
                            phase=3;Files.writeString(root.resolve("SMOKE_CAPTURE_OK.txt"),folder.toString());System.out.println("TF_SMOKE_CAPTURE_OK "+folder);client.scheduleStop();return;
                        }
                    }
                }
            }
        }catch(Exception e){phase=99;e.printStackTrace();client.scheduleStop();}
    }
}
