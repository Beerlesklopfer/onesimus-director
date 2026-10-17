// Generated protobuf/gRPC types for the Onesimus Director service (onesimus-proto).

pub mod onesimus {
    pub mod director {
        tonic::include_proto!("onesimus.director");

        /// Command parameter messages (director_commands.proto)
        pub mod commands {
            tonic::include_proto!("onesimus.director.commands");
        }

        /// Typed Director API (director_api.proto)
        pub mod api {
            tonic::include_proto!("onesimus.director.api");
        }
    }

    pub mod ccc {
        tonic::include_proto!("onesimus.ccc");
    }

    // onesimus_options.proto extends google.protobuf descriptors;
    // the extensions are available via prost descriptor reflection.
}

pub mod typed;
