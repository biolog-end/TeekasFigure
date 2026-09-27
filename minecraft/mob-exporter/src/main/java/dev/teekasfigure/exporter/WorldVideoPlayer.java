package dev.teekasfigure.exporter;

import net.minecraft.entity.*;
import net.minecraft.entity.mob.MobEntity;
import net.minecraft.entity.player.PlayerEntity;
import net.minecraft.registry.Registries;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.network.ServerPlayerEntity;
import net.minecraft.server.world.ServerWorld;
import net.minecraft.server.world.ChunkTicketType;
import net.minecraft.text.Text;
import net.minecraft.util.Identifier;
import net.minecraft.util.math.*;
import net.minecraft.world.GameMode;
import org.joml.Vector3f;
import org.joml.Quaternionf;
import java.util.*;
import java.util.concurrent.*;

/** World mutations stay bounded; cached frames are prefetched on one worker. */
public final class WorldVideoPlayer {
    private static final org.slf4j.Logger LOG=org.slf4j.LoggerFactory.getLogger("TeekasFigure");
    private static final long UPDATE_BUDGET=4_000_000;
    public record View(double x,double z,double baseY,double topY,double width,double height,SceneTimeline.View mode) {}
    public record Status(long frame,long total,int mobs,boolean paused,boolean loading,String error,View view,double actualFps,long skipped) {}
    private static final ExecutorService IO=Executors.newSingleThreadExecutor(r -> {var t=new Thread(r,"TeekasFigure timeline");t.setDaemon(true);return t;});
    private static final ChunkTicketType<UUID> TICKET=ChunkTicketType.create("teekasfigure",Comparator.comparing(UUID::toString));
    public static volatile Status status;
    private static Session session;

    private static final class Actor {
        final MobEntity mob;
        String pose;
        float scale=Float.NaN;
        Actor(MobEntity mob){this.mob=mob;}
    }
    private static final class Placement {
        final SceneTimeline.Shape shape;
        final SceneTimeline.Brush brush;
        final EntityType<?> type;
        final double x,y,z;
        final float yaw,scale;
        Actor actor;
        Placement(Session s,SceneTimeline.Frame frame,int index)throws Exception {
            shape=frame.shapes().get(index);brush=s.scene.brushes[shape.brush()];type=s.types[shape.brush()];
            scale=(float)(shape.size()/s.scene.pixelsPerBlock/brush.span());
            double theta=(s.scene.view==SceneTimeline.View.TOP?Math.IEEEremainder(shape.rotation(),Math.PI*2):0)
                +Math.toRadians(Math.IEEEremainder(brush.yaw(),360));
            yaw=(float)Math.toDegrees(theta);
            double cos=Math.cos(theta),sin=Math.sin(theta);
            if(s.scene.view==SceneTimeline.View.TOP) {
                x=s.view.x+shape.x()/s.scene.pixelsPerBlock+(brush.pivotX()*cos-brush.pivotZ()*sin)*scale;
                z=s.view.z+shape.y()/s.scene.pixelsPerBlock+(brush.pivotX()*sin+brush.pivotZ()*cos)*scale;
                y=s.view.baseY+frame.bottoms()[index]/s.scene.pixelsPerBlock-brush.minDepth()*scale;
            } else {
                x=s.view.x+shape.x()/s.scene.pixelsPerBlock-brush.pivotX()*scale;
                y=s.view.baseY+(s.scene.height-shape.y())/s.scene.pixelsPerBlock-brush.pivotY()*scale;
                // The camera is south of the stage. Larger packed depths move
                // later shapes toward it, preserving the exported layer order.
                z=s.view.z+frame.bottoms()[index]/s.scene.pixelsPerBlock-brush.minDepth()*scale;
            }
            if(!Double.isFinite(x) || !Double.isFinite(y) || !Double.isFinite(z) || !Float.isFinite(scale) || scale<=0
                || Math.abs(x)>29_900_000 || Math.abs(z)>29_900_000 || Math.abs(y)>19_000_000)
                throw new Exception("Placement exceeds safe world coordinates");
        }
    }
    private static final class Session {
        final MinecraftServer server;
        final ServerWorld world;
        final UUID owner,key=UUID.randomUUID();
        final Vec3d oldPosition;
        final float oldYaw,oldPitch;
        final GameMode oldMode;
        final boolean oldFlying;
        final SceneTimeline scene;
        final View view;
        final EntityType<?>[] types;
        final Map<Long,Actor> actors=new HashMap<>();
        final Set<ChunkPos> tickets=new HashSet<>(),requiredChunks=new HashSet<>();
        final ArrayDeque<Entity> cleanup=new ArrayDeque<>();
        Entity backdrop;
        SceneTimeline.Frame frame;
        CompletableFuture<SceneTimeline.Frame> next;
        Placement[] placements;
        int applyIndex,completedSample;
        long frameStarted=System.nanoTime(),playbackStart,pausedAt,sampleStarted,skipped;
        double actualFps;
        boolean initialized,applying=true,paused;
        Session(MinecraftServer server,ServerPlayerEntity player,SceneTimeline scene)throws Exception {
            this.server=server;world=player.getServerWorld();owner=player.getUuid();this.scene=scene;
            oldPosition=player.getPos();oldYaw=player.getYaw();oldPitch=player.getPitch();oldMode=player.interactionManager.getGameMode();
            oldFlying=player.getAbilities().flying;
            double width=scene.width/scene.pixelsPerBlock,height=scene.height/scene.pixelsPerBlock;
            // Move the whole stage, not only the camera. Ground is beyond the
            // playback far plane; Creative flight remains freely controllable.
            double base=Math.max(2048,Math.max(world.getTopY()+1024,player.getY()+4));
            double cameraDistance=Math.max(32,Math.max(width,height));
            double requiredTop=base+(scene.view==SceneTimeline.View.TOP?scene.maxStackHeight:height);
            if(requiredTop+cameraDistance+32>19_000_000)throw new Exception("Stage exceeds Minecraft coordinate safety bounds");
            double stageZ=scene.view==SceneTimeline.View.TOP?player.getZ()-height/2:player.getZ();
            if(scene.view==SceneTimeline.View.FRONT && Math.abs(stageZ+scene.maxStackHeight+cameraDistance)>29_900_000)throw new Exception("Front camera exceeds Minecraft coordinate safety bounds");
            view=new View(player.getX()-width/2,stageZ,base,requiredTop,width,height,scene.view);
            types=new EntityType<?>[scene.brushes.length];
            for(int i=0;i<types.length;i++)if(scene.brushes[i]!=null) {
                var id=Identifier.of(scene.brushes[i].type());
                if(!Registries.ENTITY_TYPE.containsId(id))throw new Exception("Unknown mob type: "+id);
                types[i]=Registries.ENTITY_TYPE.get(id);
            }
        }
        long desired(long now) {
            if(playbackStart==0)return 0;
            double elapsed=Math.max(0,(paused?pausedAt:now)-playbackStart)/1e9;
            return Math.min(scene.count-1,(long)Math.floor(elapsed*scene.fps));
        }
    }

    public static void start(MinecraftServer server,UUID owner,SceneTimeline scene){start(server,owner,scene,false);}
    public static void start(MinecraftServer server,UUID owner,SceneTimeline scene,boolean clearOthers) {
        stop(server);PlaybackWatchdog.resume();Session s=null;
        try {
            var player=server.getPlayerManager().getPlayer(owner);
            if(player==null)throw new Exception("Player left the world");
            s=new Session(server,player,scene);session=s;
            if(clearOthers)for(var world:server.getWorlds())for(var entity:world.iterateEntities())if(!(entity instanceof PlayerEntity))s.cleanup.add(entity);
            prepare(s,scene.advanceTo(0));
            LOG.info("Starting cached scene: {} frames, {} mobs peak, {} fps, stage Y {}; optional entity deletion: {}",scene.count,scene.peakMobs,scene.fps,s.view.baseY,s.cleanup.size());
            resetCamera(server);publish(s,null);
        } catch(Exception error) {
            LOG.error("Scene start failed",error);
            if(s!=null)stop(server);else {PlaybackWatchdog.disarm();try{scene.close();}catch(Exception ignored){}}
            status=new Status(0,scene.count,0,true,false,error.toString(),null,0,0);
        }
    }

    private static void prepare(Session s,SceneTimeline.Frame frame)throws Exception {
        // Preserve stable IDs first. Recycle retired actors of matching types
        // for new IDs, instead of despawning/spawning a whole new population.
        var placements=new Placement[frame.shapes().size()];
        var previous=new HashMap<>(s.actors);
        var spare=new HashMap<EntityType<?>,ArrayDeque<Actor>>();
        for(int i=0;i<placements.length;i++) {
            var p=placements[i]=new Placement(s,frame,i);
            var actor=previous.get(p.shape.id());
            if(actor!=null && !actor.mob.isRemoved() && actor.mob.getType()==p.type) {
                p.actor=actor;previous.remove(p.shape.id());
            }
        }
        for(var actor:previous.values())if(!actor.mob.isRemoved())spare.computeIfAbsent(actor.mob.getType(),k -> new ArrayDeque<>()).add(actor);
        for(var p:placements)if(p.actor==null) {
            var pool=spare.get(p.type);if(pool!=null && !pool.isEmpty())p.actor=pool.removeFirst();
        }
        // Discard only actors that cannot be reused. Finish their bounded
        // cleanup before new spawns, so the population stays within peakMobs.
        for(var pool:spare.values())for(var actor:pool)s.cleanup.add(actor.mob);
        s.actors.clear();s.requiredChunks.clear();
        s.requiredChunks.add(new ChunkPos(BlockPos.ofFloored(s.view.x,s.view.baseY,s.view.z)));
        for(var p:placements) {
            if(p.actor!=null)s.actors.put(p.shape.id(),p.actor);
            s.requiredChunks.add(new ChunkPos(BlockPos.ofFloored(p.x,p.y,p.z)));
        }
        if(s.frame!=null)s.skipped+=Math.max(0,frame.index()-s.frame.index()-1);
        s.frame=frame;s.placements=placements;s.applyIndex=0;s.applying=true;s.frameStarted=System.nanoTime();
    }

    private static void backdrop(Session s) {
        var floor=EntityType.BLOCK_DISPLAY.create(s.world);
        if(floor!=null) {
            floor.setBlockState(net.minecraft.block.Blocks.BLACK_CONCRETE.getDefaultState());
            floor.setNoGravity(true);floor.setViewRange(1024);floor.setBrightness(new net.minecraft.entity.decoration.Brightness(15,15));
            floor.setCustomName(Text.literal(Participant.PREFIX+s.key+":floor:1]"));
            if(s.view.mode==SceneTimeline.View.TOP) {
                floor.setTransformation(new AffineTransformation(new Vector3f(),new Quaternionf(),new Vector3f((float)s.view.width,0.1f,(float)s.view.height),new Quaternionf()));
                floor.setPosition(s.view.x,s.view.baseY-0.2,s.view.z);
            } else {
                floor.setTransformation(new AffineTransformation(new Vector3f(),new Quaternionf(),new Vector3f((float)s.view.width,(float)s.view.height,0.1f),new Quaternionf()));
                floor.setPosition(s.view.x,s.view.baseY,s.view.z-0.2);
            }
            s.world.spawnEntity(floor);s.backdrop=floor;
        }
    }
    public static void resetCamera(MinecraftServer server) {
        var s=session;if(s==null || s.server!=server)return;
        var player=server.getPlayerManager().getPlayer(s.owner);if(player==null)return;
        player.changeGameMode(GameMode.CREATIVE);
        double distance=Math.max(32,Math.max(s.view.width,s.view.height));
        if(s.view.mode==SceneTimeline.View.TOP)
            player.teleport(s.world,s.view.x+s.view.width/2,s.view.topY+distance,s.view.z+s.view.height/2,180,90);
        else
            player.teleport(s.world,s.view.x+s.view.width/2,s.view.baseY+s.view.height/2,s.view.z+s.scene.maxStackHeight+distance,180,0);
        player.getAbilities().flying=true;player.setVelocity(Vec3d.ZERO);player.sendAbilitiesUpdate();
    }
    public static void pause(MinecraftServer server) {
        var s=session;if(s==null || s.server!=server)return;
        long now=System.nanoTime();s.paused=!s.paused;
        if(s.playbackStart!=0) {
            if(s.paused)s.pausedAt=now;
            else {s.playbackStart+=now-s.pausedAt;s.pausedAt=0;}
        }
        s.sampleStarted=now;s.completedSample=0;s.actualFps=0;publish(s,null);
    }
    public static void ownerLeaving(MinecraftServer server,UUID owner){if(session!=null && session.server==server && session.owner.equals(owner))stop(server);}

    public static void tick(MinecraftServer server) {
        var s=session;if(s==null || s.server!=server)return;
        if(server.getPlayerManager().getPlayer(s.owner)==null){stop(server);return;}
        try {
            long now=System.nanoTime();
            if(!s.applying && !s.paused && s.next!=null && s.next.isDone()) {
                var ready=s.next.join();long desired=s.desired(now);
                if(ready.index()<=desired) {
                    s.next=null;
                    // A slow tick can make a prefetched frame obsolete. Seek
                    // straight to the latest useful frame; never pack the gap.
                    if(desired-ready.index()>Math.max(1,Math.ceil(s.scene.fps/20)))prefetch(s,desired);
                    else prepare(s,ready);
                }
            }
            long deadline=System.nanoTime()+UPDATE_BUDGET;
            while(!s.cleanup.isEmpty() && System.nanoTime()<deadline)s.cleanup.removeFirst().discard();
            if(!s.cleanup.isEmpty()){publish(s,null);return;}
            if(s.applying) {
                boolean ready=true;
                for(var chunk:s.requiredChunks)ready=requestChunk(s,chunk) && ready;
                if(!ready) {
                    if(System.nanoTime()-s.frameStarted>20_000_000_000L)throw new Exception("Stage chunks did not load within 20 seconds; playback cancelled");
                    publish(s,null);return;
                }
                if(!s.initialized){backdrop(s);s.initialized=true;}
                // Only the time budget bounds per-tick work; a fixed spawn cap
                // would delay each frame by seconds.
                while(s.applyIndex<s.placements.length && System.nanoTime()<deadline) {
                    var p=s.placements[s.applyIndex++];boolean created=p.actor==null;
                    if(created) {
                        var entity=p.type.create(s.world);
                        if(!(entity instanceof MobEntity mob))throw new Exception("Unsupported mob: "+p.brush.type());
                        mob.setAiDisabled(true);mob.setNoGravity(true);mob.noClip=true;mob.setSilent(true);mob.setInvulnerable(true);mob.setPersistent();
                        p.actor=new Actor(mob);s.actors.put(p.shape.id(),p.actor);
                    }
                    var actor=p.actor;var mob=actor.mob;
                    boolean poseChanged=!p.brush.pose().equals(actor.pose);
                    if(created || poseChanged)CapturePoses.apply(mob,p.brush.pose());
                    if(created || poseChanged || actor.scale!=p.scale) {
                        // Actor identity is stable across shape-ID reuse. Only
                        // changed scale/pose produces metadata/dimension work.
                        mob.setCustomName(Text.literal(Participant.PREFIX+s.key+":actor:"+mob.getId()+":"+p.brush.pose()+":"+p.scale+"]"));
                        mob.calculateDimensions();actor.pose=p.brush.pose();actor.scale=p.scale;
                    }
                    if(created || mob.getX()!=p.x || mob.getY()!=p.y || mob.getZ()!=p.z || mob.getYaw()!=p.yaw) {
                        mob.refreshPositionAndAngles(p.x,p.y,p.z,p.yaw,0);mob.prevYaw=p.yaw;mob.bodyYaw=p.yaw;mob.prevBodyYaw=p.yaw;mob.setHeadYaw(p.yaw);mob.prevHeadYaw=p.yaw;
                    }
                    if(created && !s.world.spawnEntity(mob))throw new Exception("Could not spawn "+p.brush.type());
                }
                if(s.applyIndex>=s.placements.length) {
                    s.applying=false;now=System.nanoTime();
                    if(s.playbackStart==0){s.playbackStart=now;s.sampleStarted=now;if(s.paused)s.pausedAt=now;}
                    else s.completedSample++;
                    long sampleElapsed=(s.paused?s.pausedAt:now)-s.sampleStarted;
                    if(sampleElapsed>=1_000_000_000L || (s.frame.index()==s.scene.count-1 && sampleElapsed>0)) {
                        s.actualFps=s.completedSample*1e9/sampleElapsed;s.completedSample=0;s.sampleStarted=now;
                    }
                }
            }
            if(s.next==null && s.frame.index()<s.scene.count-1)prefetch(s,Math.max(s.frame.index()+1,s.desired(System.nanoTime())));
            if(!s.applying && s.frame.index()==s.scene.count-1 && !s.paused){s.paused=true;s.pausedAt=System.nanoTime();}
            publish(s,null);
        } catch(Exception error){LOG.error("Scene update failed",error);String message=error.toString();stop(server);status=new Status(0,0,0,true,false,message,null,0,0);}
    }
    private static void prefetch(Session s,long index) {
        s.next=CompletableFuture.supplyAsync(() -> {try{return s.scene.advanceTo(index);}catch(Exception error){throw new CompletionException(error);}},IO);
    }
    private static boolean requestChunk(Session s,ChunkPos chunk)throws Exception {
        if(s.tickets.add(chunk)) {
            if(s.tickets.size()>192)throw new Exception("Stage exceeds safe loaded chunk budget");
            s.world.getChunkManager().addTicket(TICKET,chunk,2,s.key);
        }
        return s.world.getChunkManager().getWorldChunk(chunk.x,chunk.z)!=null;
    }
    private static void publish(Session s,String error) {
        status=new Status(s.frame.index(),s.scene.count,s.actors.size(),s.paused,s.applying || (s.next!=null && !s.next.isDone()),error,s.view,s.actualFps,s.skipped);
    }
    public static void stop(MinecraftServer server) {
        PlaybackWatchdog.disarm();var s=session;if(s==null || s.server!=server){status=null;return;}
        session=null;s.scene.cancel();
        for(var actor:s.actors.values())actor.mob.discard();
        while(!s.cleanup.isEmpty()) {
            // Optional global cleanup that has not yet run must be cancelled.
            var entity=s.cleanup.removeFirst();if(Participant.owned(entity))entity.discard();
        }
        if(s.backdrop!=null)s.backdrop.discard();
        for(var chunk:s.tickets)s.world.getChunkManager().removeTicket(TICKET,chunk,2,s.key);
        var player=server.getPlayerManager().getPlayer(s.owner);
        if(player!=null){player.changeGameMode(s.oldMode);player.teleport(s.world,s.oldPosition.x,s.oldPosition.y,s.oldPosition.z,s.oldYaw,s.oldPitch);player.getAbilities().flying=s.oldFlying && player.getAbilities().allowFlying;player.setVelocity(Vec3d.ZERO);player.sendAbilitiesUpdate();}
        LOG.info("Stopped scene: {} frames/s, {} skipped; restored player",s.actualFps,s.skipped);
        IO.execute(() -> {try{s.scene.close();}catch(Exception ignored){}});status=null;
    }
}
