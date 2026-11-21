// Generated protobuf/gRPC types for the Onesimus Director service (onesimus-proto).

pub mod onesimus {
    pub mod filed {
        tonic::include_proto!("onesimus.filed");
        pub mod plugin {
            pub mod v1 {
                tonic::include_proto!("onesimus.filed.plugin.v1");
            }
        }
        pub mod broker {
            pub mod postgresql {
                pub mod v1 {
                    tonic::include_proto!("onesimus.filed.broker.postgresql.v1");
                }
            }
            pub mod openldap {
                pub mod v1 {
                    tonic::include_proto!("onesimus.filed.broker.openldap.v1");
                }
            }
            pub mod podman {
                pub mod v1 {
                    tonic::include_proto!("onesimus.filed.broker.podman.v1");
                }
            }
        }
    }
    /// Storage Daemon service (stored_service.proto): Control and Data
    /// streams of odir and ofd to osd.
    pub mod stored {
        tonic::include_proto!("onesimus.stored");
    }
    pub mod director {
        tonic::include_proto!("onesimus.director");

        /// Usage of the month and licence state (usage.proto)
        pub mod usage {
            tonic::include_proto!("onesimus.director.usage");
        }

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
